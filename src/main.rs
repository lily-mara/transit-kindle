use axum::{
    body::{Bytes, Full},
    extract::State,
    http::StatusCode,
    response::Response,
    routing::get,
    Router,
};
use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    sync::Arc,
};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};

use chrono::{DateTime, Duration, Utc};
use eyre::{bail, eyre, Context, Result};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use skia_safe::{
    Bitmap, Canvas, Color4f, Font, FontStyle, ImageInfo, Paint, Point, TextBlob, Typeface,
};
use tokio::task::JoinSet;
use tracing_subscriber::EnvFilter;

/// unwrap an option, `continue` if it's None
macro_rules! opt_cont {
    ($opt:expr) => {
        match $opt {
            Some(x) => x,
            None => continue,
        }
    };
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StopMonitoringResponse {
    service_delivery: ServiceDelivery,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ServiceDelivery {
    stop_monitoring_delivery: StopMonitoringDelivery,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StopMonitoringDelivery {
    monitored_stop_visit: Vec<MonitoredStopVisit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MonitoredStopVisit {
    monitored_vehicle_journey: MonitoredVehicleJourney,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct MonitoredVehicleJourney {
    line_ref: Option<String>,
    direction_ref: String,
    // operator_ref: String,
    destination_name: String,
    monitored_call: MonitoredCall,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct MonitoredCall {
    expected_arrival_time: Option<String>,
    stop_point_ref: String,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Upcoming {
    time: DateTime<Utc>,
}

struct UpcomingResponse {
    agency: String,
    upcoming: BTreeMap<Line, Vec<Upcoming>>,
}

#[derive(Clone)]
struct AppState {
    client: Client,
    config_file: ConfigFile,
}

#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Line {
    line: String,
    agency: String,
    direction: String,
    destination: String,
}

#[derive(Clone)]
struct Client {
    api_key: Arc<str>,
    destination_subs: Arc<HashMap<String, String>>,
}

#[derive(Deserialize, Clone)]
struct ConfigFile {
    stops: Vec<StopConfig>,
    destination_subs: HashMap<String, String>,
    layout: LayoutConfig,
    api_key: String,
}

#[derive(Deserialize, Clone)]
struct LayoutConfig {
    left: SideConfig,
    right: SideConfig,
    width: i32,
    height: i32,
}

#[derive(Deserialize, Clone)]
struct SideConfig {
    sections: Vec<SectionConfig>,
}

#[derive(Deserialize, Clone)]
struct SectionConfig {
    agency: String,
    direction: String,
}

#[derive(Deserialize, Clone)]
struct StopConfig {
    agency: String,
    #[serde(default)]
    line_prefix_subs: HashMap<String, String>,
    stops: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    journeys: Vec<MonitoredVehicleJourney>,
    live_time: DateTime<Utc>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config_file = serde_yaml::from_reader::<_, ConfigFile>(std::fs::File::open("stops.yml")?)?;

    let client = Client::new(
        config_file.api_key.clone(),
        config_file.destination_subs.clone(),
    );

    let app = Router::new()
        .route("/stops.png", get(render_page))
        .route("/error.png", get(render_error_page))
        .with_state(AppState {
            client,
            config_file,
        })
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));

    info!(port = 3001, "listening!");

    axum::Server::bind(&"0.0.0.0:3001".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

async fn render_page(State(state): State<AppState>) -> Response<Full<Bytes>> {
    let mut status = StatusCode::OK;
    let data = match render_png(state.client, &state.config_file).await {
        Ok(x) => x,
        Err(e) => {
            warn!(error=?e, "error rendering stop data");
            status = StatusCode::INTERNAL_SERVER_ERROR;
            render_err_png(&state.config_file, format!("{}", e)).unwrap()
        }
    };

    Response::builder()
        .status(status)
        .header("Content-Type", "image/png")
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}

async fn render_error_page(State(state): State<AppState>) -> Response<Full<Bytes>> {
    let data = render_err_png(&state.config_file, format!("idk")).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}

fn render_err_png(config_file: &ConfigFile, error: String) -> Result<Vec<u8>> {
    let black_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);

    let typeface = Typeface::new("arial", FontStyle::normal())
        .ok_or(eyre!("failed to construct skia typeface"))?;

    let font = Font::new(typeface, 36.0);

    let failure_blob = TextBlob::new("FAILED TO RENDER", &font)
        .ok_or(eyre!("failed to construct skia text blob"))?;

    let error_blob =
        TextBlob::new(error, &font).ok_or(eyre!("failed to construct skia text blob"))?;

    let data = render_ctx(config_file, move |canvas| {
        canvas.draw_text_blob(failure_blob, (100, 200), &black_paint);
        canvas.draw_text_blob(error_blob, (100, 250), &black_paint);
        Ok(())
    })?;

    Ok(data)
}

fn render_ctx(
    config_file: &ConfigFile,
    closure: impl FnOnce(&mut Canvas) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut bitmap = Bitmap::new();
    if !bitmap.set_info(
        &ImageInfo::new(
            (config_file.layout.width, config_file.layout.height),
            skia_safe::ColorType::Gray8,
            skia_safe::AlphaType::Unknown,
            None,
        ),
        None,
    ) {
        bail!("failed to initialize skia bitmap");
    }
    bitmap.alloc_pixels();

    let mut canvas =
        Canvas::from_bitmap(&bitmap, None).ok_or(eyre!("failed to construct skia canvas"))?;

    canvas.clear(Color4f::new(1.0, 1.0, 1.0, 1.0));

    closure(&mut canvas)?;

    let image = bitmap.as_image();

    let mut rotated_bitmap = Bitmap::new();
    if !rotated_bitmap.set_info(
        &ImageInfo::new(
            (config_file.layout.height, config_file.layout.width),
            skia_safe::ColorType::Gray8,
            skia_safe::AlphaType::Unknown,
            None,
        ),
        None,
    ) {
        bail!("failed to initialize skia bitmap");
    }
    rotated_bitmap.alloc_pixels();

    let mut rotated_canvas = Canvas::from_bitmap(&rotated_bitmap, None)
        .ok_or(eyre!("failed to construct skia canvas"))?;

    rotated_canvas.translate(Point::new(config_file.layout.height as f32, 0.0));
    rotated_canvas.rotate(90.0, Some(Point::new(0.0, 0.0)));
    rotated_canvas.draw_image(image, (0, 0), None);

    let rotated_image_data = rotated_bitmap
        .as_image()
        .encode(None, skia_safe::EncodedImageFormat::PNG, None)
        .ok_or(eyre!("failed to encode skia image"))?;

    Ok(rotated_image_data.as_bytes().into())
}

async fn render_png(client: Client, config_file: &ConfigFile) -> Result<Vec<u8>> {
    let stop_data = load_stop_data(client, config_file.clone()).await?;

    let black_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    let grey_paint = Paint::new(Color4f::new(0.8, 0.8, 0.8, 1.0), None);

    let typeface = Typeface::new("arial", FontStyle::normal())
        .ok_or(eyre!("failed to construct skia typeface"))?;

    let font = Font::new(typeface, 18.0);

    let draw_data = |canvas: &mut Canvas,
                     section: &SectionConfig,
                     (x1, x2): (i32, i32),
                     y: &mut i32|
     -> Result<()> {
        let agency = match stop_data.get(&section.agency) {
            Some(x) => x,
            None => {
                warn!(agency = &section.agency, "missing data for expected agency");
                return Ok(());
            }
        };

        let lines = match agency.get(&section.direction) {
            Some(x) => x,
            None => {
                warn!(
                    agency = &section.agency,
                    direction = &section.direction,
                    "missing data for expected direction within agency"
                );
                return Ok(());
            }
        };

        if x1 > 0 {
            canvas.draw_line((x1, 0), (x1, config_file.layout.height), &black_paint);
        }

        for (line, upcoming) in lines {
            let x = x1 + 20;

            let line_name_blob = TextBlob::new(&line.line, &font)
                .ok_or(eyre!("failed to construct skia text blob"))?;

            let line_name_bounds = line_name_blob.bounds();

            let line_name_oval = line_name_bounds.with_offset((x, *y));

            canvas.draw_oval(line_name_oval, &grey_paint);

            canvas.draw_text_blob(&line_name_blob, (x, *y), &black_paint);

            let destination_blob = TextBlob::new(&line.destination, &font)
                .ok_or(eyre!("failed to construct skia text blob"))?;
            canvas.draw_text_blob(
                destination_blob,
                ((x + line_name_bounds.width() as i32), *y),
                &black_paint,
            );

            let mins = upcoming.into_iter().map(|t| t.minutes()).join(", ");
            let time_text = format!("{mins} mins");

            let time_blob = TextBlob::new(time_text, &font)
                .ok_or(eyre!("failed to construct skia text blob"))?;

            let x = x2 - time_blob.bounds().width() as i32;
            canvas.draw_text_blob(time_blob, (x, *y), &black_paint);

            *y += 40;
        }

        canvas.draw_line((x1, *y), (x2, *y), &black_paint);
        *y += 28;

        Ok(())
    };

    let halfway = config_file.layout.width / 2;

    let image_data = render_ctx(config_file, |canvas| {
        let mut y = 38;
        for section in &config_file.layout.left.sections {
            draw_data(canvas, section, (0, halfway), &mut y)?;
        }

        let mut y = 38;
        for section in &config_file.layout.right.sections {
            draw_data(canvas, section, (halfway, config_file.layout.width), &mut y)?;
        }

        Ok(())
    })?;

    Ok(image_data)
}

async fn load_stop_data(
    client: Client,
    config_file: ConfigFile,
) -> Result<HashMap<String, HashMap<String, Vec<(Line, Vec<Upcoming>)>>>> {
    let mut joinset = JoinSet::new();

    for agency in config_file.stops {
        let client = client.clone();
        joinset.spawn(async move {
            client
                .load_upcoming(agency.clone())
                .await
                .wrap_err_with(|| format!("loading data for agency {}", agency.agency))
        });
    }

    let mut agency_direction_to_departures = HashMap::<String, HashMap<String, Vec<_>>>::new();

    while let Some(result) = joinset.join_next().await {
        let response = result??;

        for (line, upcoming) in response.upcoming {
            agency_direction_to_departures
                .entry(response.agency.clone())
                .or_default()
                .entry(line.direction.clone())
                .or_default()
                .push((line, upcoming));
        }
    }

    Ok(agency_direction_to_departures)
}

impl Client {
    fn new(api_key: String, destination_subs: HashMap<String, String>) -> Self {
        Self {
            api_key: Arc::from(api_key),
            destination_subs: Arc::new(destination_subs),
        }
    }

    fn load_cached(path: String) -> Result<Vec<MonitoredVehicleJourney>> {
        debug!(path, "trying to load cached file");
        let file = std::fs::File::open(&path)?;
        let cached: Cached = serde_json::from_reader(file)?;

        let age = Utc::now() - cached.live_time;
        debug!(path, ?age, "loaded cached data");
        if age > Duration::minutes(5) {
            debug!(path, ?age, "skipping cached data because age");
            bail!("cached data was old");
        }

        debug!(path, "using cached data");

        Ok(cached.journeys)
    }

    fn store_cache(path: String, journeys: Vec<MonitoredVehicleJourney>) -> Result<()> {
        let cached = Cached {
            journeys,
            live_time: Utc::now(),
        };

        debug!(path, "storing cache");

        let file = std::fs::File::create(&path)?;

        serde_json::to_writer(file, &cached)?;

        debug!(path, "cache ok");

        Ok(())
    }

    async fn request_with_caching(
        &self,
        agency: &str,
        stops: &[String],
    ) -> Result<Vec<MonitoredVehicleJourney>> {
        let url_path = format!(".cache-{agency}.json");

        let url_path2 = url_path.clone();

        if let Ok(data) = tokio::task::spawn_blocking(move || Self::load_cached(url_path2)).await? {
            return Ok(data);
        }

        let url = format!(
            "https://api.511.org/transit/StopMonitoring?api_key={api_key}&agency={agency}&format=json",
            api_key=self.api_key,
        );

        let response = reqwest::get(url).await?.error_for_status()?;

        let text = response.text().await?;

        let bom = unicode_bom::Bom::from(text.as_bytes());

        let stripped_response = &text[bom.len()..];

        let url_path2 = url_path.clone();

        let jd = &mut serde_json::Deserializer::from_str(stripped_response);
        let json: StopMonitoringResponse = serde_path_to_error::deserialize(jd)?;

        let journeys = json
            .service_delivery
            .stop_monitoring_delivery
            .monitored_stop_visit
            .into_iter()
            .filter_map(|visit| {
                if stops.contains(
                    &visit
                        .monitored_vehicle_journey
                        .monitored_call
                        .stop_point_ref,
                ) {
                    Some(visit.monitored_vehicle_journey)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let journeys2 = journeys.clone();

        if let Err(e) =
            tokio::task::spawn_blocking(move || Self::store_cache(url_path2, journeys2)).await?
        {
            warn!(error = ?e, path=url_path,"failed to cache data");
        }

        Ok(journeys)
    }

    async fn load_upcoming(self, stop_config: StopConfig) -> Result<UpcomingResponse> {
        let agency = stop_config.agency;
        let stops = stop_config.stops;

        let journeys = self.request_with_caching(&agency, &stops).await?;

        let mut upcoming = BTreeMap::<_, Vec<_>>::new();

        for journey in journeys {
            let expected_arrival_time = opt_cont!(&journey.monitored_call.expected_arrival_time);
            let line = opt_cont!(&journey.line_ref);

            let time = expected_arrival_time.parse::<DateTime<Utc>>()?;

            if time < Utc::now() {
                continue;
            }

            let destination = self
                .destination_subs
                .get(&*journey.destination_name)
                .map(|d| d)
                .unwrap_or(&journey.destination_name)
                .clone();

            let mut line = line.clone();
            for (prefix, replacement) in &stop_config.line_prefix_subs {
                if line.starts_with(prefix) {
                    line = replacement.clone();
                    break;
                }
            }

            upcoming
                .entry(Line {
                    line,
                    destination,
                    agency: agency.clone(),
                    direction: journey.direction_ref.clone(),
                })
                .or_default()
                .push(Upcoming { time })
        }

        for times in upcoming.values_mut() {
            times.sort();
            if times.len() > 4 {
                for _ in times.drain(4..) {}
            }
        }

        Ok(UpcomingResponse { agency, upcoming })
    }
}

impl Upcoming {
    fn minutes(&self) -> i64 {
        (self.time - Utc::now()).num_minutes()
    }
}
