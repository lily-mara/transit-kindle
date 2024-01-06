use std::collections::HashMap;

use chrono::prelude::*;
use eyre::{bail, Result};
use itertools::Itertools;
use tracing::warn;

use crate::{
    api_client::{StopData, Upcoming},
    config::{ConfigFile, SectionConfig, SideConfig, TextSectionConfig},
};

pub struct Layout {
    pub left: Column,
    pub right: Column,

    /// Mapping of names of agencies to the timestamp that their data was last refreshed
    pub all_agencies: HashMap<String, DateTime<Utc>>,
}

pub struct Column {
    pub rows: Vec<Row>,
}

pub enum Row {
    Agency(Agency),
    Text(String),
}

pub struct Agency {
    pub lines: Vec<Line>,
}

pub struct Line {
    pub id: String,
    pub destination: String,
    pub departure_minutes: Vec<i64>,
}

impl Line {
    pub fn departure_minutes_str(&self) -> String {
        self.departure_minutes.iter().join(", ")
    }
}

pub fn data_to_layout(stop_data: StopData, config_file: &ConfigFile) -> Layout {
    let mut all_agencies = HashMap::new();

    let left = column(&stop_data, &config_file.layout.left, &mut all_agencies);
    let right = column(&stop_data, &config_file.layout.right, &mut all_agencies);

    Layout {
        left,
        right,
        all_agencies,
    }
}

fn column(
    stop_data: &StopData,
    side: &SideConfig,
    all_agencies: &mut HashMap<String, DateTime<Utc>>,
) -> Column {
    let mut rows = Vec::new();

    for section in &side.sections {
        match section {
            SectionConfig::AgencySection(agency_section) => {
                match agency(
                    stop_data,
                    &agency_section.agency,
                    &agency_section.direction,
                    all_agencies,
                ) {
                    Ok(x) => rows.push(Row::Agency(x)),
                    Err(e) => {
                        warn!(error = %e, "failed to generate agency data");
                    }
                }
            }
            SectionConfig::TextSection(TextSectionConfig { text }) => {
                rows.push(Row::Text(text.clone()));
            }
        }
    }

    Column { rows }
}

fn agency(
    stop_data: &StopData,
    agency_name: &str,
    direction: &str,
    all_agencies: &mut HashMap<String, DateTime<Utc>>,
) -> Result<Agency> {
    let agency = match stop_data.agencies.get(agency_name) {
        Some(x) => x,
        None => {
            bail!("agency {} not found in API response data", agency_name);
        }
    };

    all_agencies.insert(agency_name.to_owned(), agency.live_time);

    let lines_in = match agency.directions.get(direction) {
        Some(x) => x,
        None => {
            bail!(
                "agency {} did not contain direction {}",
                agency_name,
                direction
            );
        }
    };

    let mut lines = Vec::new();

    for (line, upcoming) in &lines_in.lines {
        lines.push(Line {
            id: line.line.clone(),
            destination: line.destination.clone(),
            departure_minutes: upcoming.iter().map(Upcoming::minutes).collect(),
        })
    }

    Ok(Agency { lines })
}
