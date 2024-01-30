use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct ConfigFile {
    pub stops: Vec<StopConfig>,
    #[serde(default)]
    pub destination_subs: HashMap<String, String>,
    pub layout: LayoutConfig,
    pub api_key: String,
}

#[derive(Deserialize, Clone)]
pub struct LayoutConfig {
    pub left: SideConfig,
    pub right: SideConfig,
    pub width: i32,
    pub height: i32,
}

#[derive(Deserialize, Clone)]
pub struct SideConfig {
    pub sections: Vec<SectionConfig>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub enum SectionConfig {
    AgencySection(AgencySectionConfig),
    TextSection(TextSectionConfig),
}

#[derive(Deserialize, Clone)]
pub struct TextSectionConfig {
    pub text: String,
}

#[derive(Deserialize, Clone)]
pub struct AgencySectionConfig {
    pub agency: String,
    pub direction: String,
}

#[derive(Deserialize, Clone)]
pub struct StopConfig {
    pub agency: String,
    #[serde(default)]
    pub line_prefix_subs: HashMap<String, String>,
    pub stops: Vec<String>,
}
