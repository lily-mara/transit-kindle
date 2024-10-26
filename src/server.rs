use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, Redirect},
    routing::get,
    Router,
};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    api_client::DataAccess, config::ConfigFile, html::stops_html, layout::data_to_layout,
    render::SharedRenderData,
};

#[derive(Clone)]
struct AppState {
    data_access: Arc<DataAccess>,
    config_file: ConfigFile,
}

pub async fn serve(
    data_access: Arc<DataAccess>,
    shared_render_data: Arc<SharedRenderData>,
    config_file: ConfigFile,
) -> eyre::Result<()> {
    let app = kindling::ApplicationBuilder::new(Router::new(), "http://localhost:3001")
        .add_handler(
            "/stops.png",
            crate::handler::TransitHandler {
                shared: shared_render_data,
                data_access: data_access.clone(),
                config_file: config_file.clone(),
            },
        )
        .attach()
        .route("/stops.html", get(handle_stops_html))
        .with_state(AppState {
            data_access,
            config_file,
        })
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));

    let listener = TcpListener::bind(&"0.0.0.0:3001").await?;

    info!(port = 3001, "listening!");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn handle_stops_html(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let stop_data = state
        .data_access
        .load_stop_data(state.config_file.clone())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let layout = data_to_layout(stop_data, &state.config_file);

    let html = stops_html(layout).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(html))
}
