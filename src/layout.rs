use std::collections::HashMap;

use eyre::{bail, Result};
use itertools::Itertools;
use tracing::warn;

use crate::{
    api_client::{self, Upcoming},
    config::{ConfigFile, SideConfig},
};

pub struct Layout {
    pub left: Column,
    pub right: Column,
}

pub struct Column {
    pub agencies: Vec<Agency>,
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

pub fn data_to_layout(
    stop_data: HashMap<String, HashMap<String, Vec<(api_client::Line, Vec<api_client::Upcoming>)>>>,
    config_file: &ConfigFile,
) -> Layout {
    let left = column(&stop_data, &config_file.layout.left);
    let right = column(&stop_data, &config_file.layout.right);

    Layout { left, right }
}

fn column(
    stop_data: &HashMap<
        String,
        HashMap<String, Vec<(api_client::Line, Vec<api_client::Upcoming>)>>,
    >,
    side: &SideConfig,
) -> Column {
    let mut agencies = Vec::new();

    for section in &side.sections {
        match agency(stop_data, &section.agency, &section.direction) {
            Ok(x) => agencies.push(x),
            Err(e) => {
                warn!(error = %e, "failed to generate agency data");
            }
        }
    }

    Column { agencies }
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
