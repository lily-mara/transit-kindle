use std::sync::Arc;

use axum::Router;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{api_client::DataAccess, config::ConfigFile, render::SharedRenderData};

pub async fn serve(
    data_access: Arc<DataAccess>,
    shared_render_data: Arc<SharedRenderData>,
    config_file: ConfigFile,
) -> eyre::Result<()> {
    let app = kindling::ApplicationBuilder::new(Router::new(), "http://transit.lilys.hair")
        .add_handler(
            "/stops.png",
            crate::handler::TransitHandler {
                shared: shared_render_data,
                data_access: data_access.clone(),
                config_file: config_file.clone(),
            },
        )
        .attach()
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));

    let listener = TcpListener::bind(&"0.0.0.0:3001").await?;

    info!(port = 3001, "listening!");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}
