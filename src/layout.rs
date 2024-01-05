use std::collections::HashMap;

use eyre::{bail, Result};
use itertools::Itertools;
use tracing::warn;

use crate::{
    api_client::{self, StopData, Upcoming},
    config::{ConfigFile, SectionConfig, SideConfig, TextSectionConfig},
};

pub struct Layout {
    pub left: Column,
    pub right: Column,
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
    let left = column(&stop_data, &config_file.layout.left);
    let right = column(&stop_data, &config_file.layout.right);

    Layout { left, right }
}

fn column(stop_data: &StopData, side: &SideConfig) -> Column {
    let mut rows = Vec::new();

    for section in &side.sections {
        match section {
            SectionConfig::AgencySection(agency_section) => {
                match agency(stop_data, &agency_section.agency, &agency_section.direction) {
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
    stop_data: &HashMap<
        String,
        HashMap<String, Vec<(api_client::Line, Vec<api_client::Upcoming>)>>,
    >,
    agency_name: &str,
    direction: &str,
) -> Result<Agency> {
    let agency = match stop_data.get(agency_name) {
        Some(x) => x,
        None => {
            bail!("agency {} not found in API response data", agency_name);
        }
    };

    let lines_in = match agency.get(direction) {
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

    for (line, upcoming) in lines_in {
        lines.push(Line {
            id: line.line.clone(),
            destination: line.destination.clone(),
            departure_minutes: upcoming.iter().map(Upcoming::minutes).collect(),
        })
    }

    Ok(Agency { lines })
}
