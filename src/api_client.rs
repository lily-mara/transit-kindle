use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use chrono::{DateTime, Duration, Utc};
use eyre::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::config::{ConfigFile, StopConfig};

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
    direction_ref: Option<String>,
    destination_name: Option<String>,
    monitored_call: MonitoredCall,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct MonitoredCall {
    expected_arrival_time: Option<String>,
    stop_point_ref: String,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Upcoming {
    time: DateTime<Utc>,
}

struct UpcomingResponse {
    agency: String,
    upcoming: BTreeMap<Line, Vec<Upcoming>>,
}

#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Line {
    pub line: String,
    pub agency: String,
    pub direction: String,
    pub destination: String,
}

#[derive(Clone)]
pub struct Client {
    api_key: Arc<str>,
    destination_subs: Arc<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    journeys: Vec<MonitoredVehicleJourney>,
    live_time: DateTime<Utc>,
}

pub type StopData = HashMap<String, HashMap<String, Vec<(Line, Vec<Upcoming>)>>>;

impl Client {
    pub fn new(api_key: String, destination_subs: HashMap<String, String>) -> Self {
        Self {
            api_key: Arc::from(api_key),
            destination_subs: Arc::new(destination_subs),
        }
    }

    pub async fn load_stop_data(self, config_file: ConfigFile) -> Result<StopData> {
        let mut joinset = JoinSet::new();

        for agency in config_file.stops {
            let client = self.clone();
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
            let direction = opt_cont!(&journey.direction_ref);
            let destination = opt_cont!(&journey.destination_name);

            let time = expected_arrival_time.parse::<DateTime<Utc>>()?;

            if time < Utc::now() {
                continue;
            }

            let destination = self
                .destination_subs
                .get(destination)
                .map(|d| d)
                .unwrap_or(destination)
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
                    direction: direction.clone(),
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
    pub fn minutes(&self) -> i64 {
        (self.time - Utc::now()).num_minutes()
    }
}
