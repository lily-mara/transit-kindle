use api_client::DataAccess;
use eyre::Result;
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
mod html;
mod layout;
mod png;
mod server;

use crate::config::*;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .without_time()
        .init();

    let config_file = serde_yaml::from_reader::<_, ConfigFile>(std::fs::File::open("stops.yml")?)?;

    if std::env::var("TEST_CONFIG").is_ok() {
        return Ok(());
    }

    let data_access = DataAccess::new(config_file.clone());

    server::serve(data_access, config_file).await?;

    Ok(())
}
