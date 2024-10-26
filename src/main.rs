use api_client::DataAccess;
use eyre::Result;
use render::SharedRenderData;
use std::io::IsTerminal;
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

mod agencies;
mod api_client;
mod config;
mod handler;
mod html;
mod layout;
mod render;
mod server;

use crate::config::*;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_ansi(std::io::stdout().is_terminal())
        .init();

    let config_file = serde_yaml::from_reader::<_, ConfigFile>(std::fs::File::open("stops.yml")?)?;

    if std::env::var("TEST_CONFIG").is_ok() {
        return Ok(());
    }

    let data_access = DataAccess::new(config_file.clone());
    let shared_render_data = SharedRenderData::new();

    server::serve(data_access, shared_render_data, config_file).await?;

    Ok(())
}
