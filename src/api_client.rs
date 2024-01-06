use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use eyre::{Context, Result};
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
    live_time: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Line {
    pub line: String,
    pub agency: String,
    pub direction: String,
    pub destination: String,
}

pub struct Client {
    api_key: Arc<str>,
    destination_subs: Arc<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    journeys: Vec<MonitoredVehicleJourney>,
    live_time: DateTime<Utc>,
}

#[derive(Default)]
pub struct StopData {
    pub agencies: HashMap<String, AgencyDirections>,
}

#[derive(Default)]
pub struct AgencyDirections {
    pub live_time: DateTime<Utc>,
    pub directions: HashMap<String, AgencyDirectionLines>,
}

#[derive(Default)]
pub struct AgencyDirectionLines {
    pub lines: Vec<(Line, Vec<Upcoming>)>,
}

pub struct DataAccess {
    client: Arc<Client>,
}

impl DataAccess {
    pub fn new(config_file: ConfigFile) -> Arc<Self> {
        let access = Self {
            client: Arc::new(Client::new(
                config_file.api_key.clone(),
                config_file.destination_subs.clone(),
            )),
        };

        let client = access.client.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = client.load_stop_data(config_file.clone()).await {
                    warn!(?e, "failed to load stop data")
                }
                tokio::time::sleep(std::time::Duration::from_secs(60 * 3)).await;
            }
        });

        Arc::new(access)
    }

    pub async fn load_stop_data(&self, config_file: ConfigFile) -> Result<StopData> {
        let mut joinset = JoinSet::new();

        for agency in config_file.stops {
            let client = self.client.clone();
            joinset.spawn(async move {
                client
                    .load_upcoming_from_cache(agency.clone())
                    .await
                    .wrap_err_with(|| format!("loading data for agency {}", agency.agency))
            });
        }

        let mut data = StopData {
            agencies: HashMap::new(),
        };

        while let Some(result) = joinset.join_next().await {
            let response = result??;

            for (line, upcoming) in response.upcoming {
                let agency_directions = data.agencies.entry(response.agency.clone()).or_default();

                agency_directions.live_time = response.live_time;

                agency_directions
                    .directions
                    .entry(line.direction.clone())
                    .or_default()
                    .lines
                    .push((line, upcoming));
            }
        }

        Ok(data)
    }
}

impl Client {
    pub fn new(api_key: String, destination_subs: HashMap<String, String>) -> Self {
        Self {
            api_key: Arc::from(api_key),
            destination_subs: Arc::new(destination_subs),
        }
    }

    async fn load_stop_data(self: &Arc<Self>, config_file: ConfigFile) -> Result<()> {
        let mut joinset = JoinSet::new();

        for StopConfig { agency, stops, .. } in config_file.stops {
            let client = self.clone();
            joinset.spawn(async move {
                client
                    .request_and_cache(&agency, &stops)
                    .await
                    .wrap_err_with(|| format!("loading data for agency {}", agency))
            });
        }

        while let Some(result) = joinset.join_next().await {
            result??;
        }

        Ok(())
    }

    fn load_cached(path: &str) -> Result<Cached> {
        debug!(path, "trying to load cached file");
        let file = std::fs::File::open(&path)?;
        let cached: Cached = serde_json::from_reader(file)?;

        let age = Utc::now() - cached.live_time;
        debug!(path, ?age, "using cached data");

        Ok(cached)
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

    fn cache_path(agency: &str) -> String {
        format!(".cache-{agency}.json")
    }

    async fn load_upcoming_from_cache(&self, stop_config: StopConfig) -> Result<UpcomingResponse> {
        let cache_path = Self::cache_path(&stop_config.agency);

        let journeys =
            tokio::task::spawn_blocking(move || Self::load_cached(&cache_path)).await??;

        let upcoming = self.transform_results(&stop_config, journeys)?;

        Ok(upcoming)
    }

    async fn request_and_cache(
        &self,
        agency: &str,
        stops: &[String],
    ) -> Result<Vec<MonitoredVehicleJourney>> {
        let url = format!(
            "https://api.511.org/transit/StopMonitoring?api_key={api_key}&agency={agency}&format=json",
            api_key=self.api_key,
        );

        let response = reqwest::get(url).await?.error_for_status()?;

        let text = response.text().await?;

        let bom = unicode_bom::Bom::from(text.as_bytes());

        let stripped_response = &text[bom.len()..];

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

        let cache_path = Self::cache_path(agency);

        if let Err(e) =
            tokio::task::spawn_blocking(move || Self::store_cache(cache_path, journeys2)).await?
        {
            warn!(error = ?e, path=Self::cache_path(agency), "failed to cache data");
        }

        Ok(journeys)
    }

    fn transform_results(
        &self,
        stop_config: &StopConfig,
        cached: Cached,
    ) -> Result<UpcomingResponse> {
        let mut upcoming = BTreeMap::<_, Vec<_>>::new();

        for journey in cached.journeys {
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
                    agency: stop_config.agency.clone(),
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

        Ok(UpcomingResponse {
            agency: stop_config.agency.clone(),
            upcoming,
            live_time: cached.live_time,
        })
    }
}

impl Upcoming {
    pub fn minutes(&self) -> i64 {
        (self.time - Utc::now()).num_minutes()
    }
}
