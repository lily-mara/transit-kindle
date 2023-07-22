use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    sync::Arc,
};
use tracing::{debug, warn};

use chrono::{DateTime, Duration, Utc};
use eyre::{bail, eyre, Context, Result};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use skia_safe::{
    Bitmap, Canvas, Color4f, Font, FontStyle, ImageInfo, Paint, Rect, TextBlob, Typeface,
};
use tokio::task::JoinSet;
use tracing_subscriber::EnvFilter;

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
    line_ref: String,
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

struct Response {
    agency: String,
    upcoming: BTreeMap<Line, Vec<Upcoming>>,
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

    let stop_data = load_stop_data(client, config_file.clone()).await?;

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

    let black_paint = Paint::new(Color4f::new(0.0, 0.0, 0.0, 1.0), None);
    let grey_paint = Paint::new(Color4f::new(0.8, 0.8, 0.8, 1.0), None);

    let typeface = Typeface::new("arial", FontStyle::normal())
        .ok_or(eyre!("failed to construct skia typeface"))?;

    let font = Font::new(typeface, 18.0);

    let mut draw_data =
        |section: &SectionConfig, (x1, x2): (i32, i32), y: &mut i32| -> Result<()> {
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

                let line_name_oval = Rect::new(
                    x as f32 + line_name_bounds.left + 5.0,
                    *y as f32 + line_name_bounds.top,
                    x as f32 + line_name_bounds.width() - 28.0,
                    *y as f32 + line_name_bounds.height() - 18.0,
                );

                canvas.draw_oval(line_name_oval, &grey_paint);
                canvas.draw_text_blob(&line_name_blob, (x, *y), &black_paint);

                let destination_blob = TextBlob::new(&line.destination, &font)
                    .ok_or(eyre!("failed to construct skia text blob"))?;
                canvas.draw_text_blob(
                    destination_blob,
                    ((x + line_name_bounds.width() as i32 - 20), *y),
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

    let mut y = 38;
    for section in &config_file.layout.left.sections {
        draw_data(section, (0, halfway), &mut y)?;
    }

    let mut y = 38;
    for section in &config_file.layout.right.sections {
        draw_data(section, (halfway, config_file.layout.width), &mut y)?;
    }

    let image = bitmap
        .as_image()
        .encode(None, skia_safe::EncodedImageFormat::PNG, None)
        .ok_or(eyre!("failed to encode skia image"))?;

    std::fs::write("image.png", &*image)?;

    Ok(())
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
                .load_upcoming(agency.agency.clone(), agency.stops)
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

        let json = serde_json::from_str::<StopMonitoringResponse>(stripped_response)?;

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

    async fn load_upcoming(
        self,
        agency: impl Into<String>,
        stops: Vec<String>,
    ) -> Result<Response> {
        let agency = agency.into();

        let journeys = self.request_with_caching(&agency, &stops).await?;

        let mut upcoming = BTreeMap::<_, Vec<_>>::new();

        for journey in journeys {
            let expected_arrival_time = match &journey.monitored_call.expected_arrival_time {
                Some(x) => x,
                None => continue,
            };

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

            upcoming
                .entry(Line {
                    line: journey.line_ref.clone(),
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

        Ok(Response { agency, upcoming })
    }
}

impl Upcoming {
    fn minutes(&self) -> i64 {
        (self.time - Utc::now()).num_minutes()
    }
}
