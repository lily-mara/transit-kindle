use api_client::Client;
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

mod api_client;
mod config;
mod render;
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

    let client = Client::new(
        config_file.api_key.clone(),
        config_file.destination_subs.clone(),
    );

    server::serve(client, config_file).await?;

    Ok(())
}
