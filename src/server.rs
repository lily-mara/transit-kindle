use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use eyre::Context;
use serde::Deserialize;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    api_client::DataAccess,
    config::ConfigFile,
    html::stops_html,
    layout::data_to_layout,
    png::{self, RenderTarget, SharedRenderData},
};

#[derive(Clone)]
struct AppState {
    data_access: Arc<DataAccess>,
    shared_render_data: Arc<SharedRenderData>,
    config_file: ConfigFile,
}

struct ErrorPng {
    data: Vec<u8>,
}

trait WrapErrPng<T> {
    fn wrap_err_png(
        self,
        render_target: RenderTarget,
        shared: &Arc<SharedRenderData>,
        config_file: &ConfigFile,
    ) -> Result<T, ErrorPng>;
}

impl<T> WrapErrPng<T> for eyre::Result<T> {
    fn wrap_err_png(
        self,
        render_target: RenderTarget,
        shared: &Arc<SharedRenderData>,
        config_file: &ConfigFile,
    ) -> Result<T, ErrorPng> {
        match self {
            Ok(x) => Ok(x),
            Err(error) => Err(ErrorPng {
                data: png::error_png(render_target, shared.clone(), config_file, error).unwrap(),
            }),
        }
    }
}

impl IntoResponse for ErrorPng {
    fn into_response(self) -> Response {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("Content-Type", "image/png")
            .body(Body::from(Bytes::from(self.data)))
            .unwrap()
            .into_response()
    }
}

pub async fn serve(
    data_access: Arc<DataAccess>,
    shared_render_data: Arc<SharedRenderData>,
    config_file: ConfigFile,
) -> eyre::Result<()> {
    let app = Router::new()
        .route("/stops.png", get(handle_stops_png))
        .route("/black.png", get(handle_black_png))
        .route("/stops.html", get(handle_stops_html))
        .route("/", get(handle_index))
        .with_state(AppState {
            data_access,
            shared_render_data,
            config_file,
        })
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));

    let listener = TcpListener::bind(&"0.0.0.0:3001").await?;

    info!(port = 3001, "listening!");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn handle_index() -> Redirect {
    Redirect::temporary("/stops.html")
}

async fn handle_stops_png(
    State(state): State<AppState>,
    target: Option<Query<ImageTarget>>,
) -> Result<Response<Body>, ErrorPng> {
    let render_target = target.map(|t| t.0.target).unwrap_or(RenderTarget::Browser);

    let stop_data = state
        .data_access
        .load_stop_data(state.config_file.clone())
        .await
        .wrap_err("load stop data")
        .wrap_err_png(render_target, &state.shared_render_data, &state.config_file)?;

    let layout = data_to_layout(stop_data, &state.config_file);

    let data = png::stops_png(
        render_target,
        state.shared_render_data.clone(),
        layout,
        &state.config_file,
    )
    .wrap_err("render schedule")
    .wrap_err_png(render_target, &state.shared_render_data, &state.config_file)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .body(Body::from(Bytes::from(data)))
        .unwrap())
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

#[derive(Deserialize)]
struct ImageTarget {
    target: RenderTarget,
}

async fn handle_black_png(
    State(state): State<AppState>,
    target: Option<Query<ImageTarget>>,
) -> Result<Response<Body>, ErrorPng> {
    let render_target = target.map(|t| t.0.target).unwrap_or(RenderTarget::Browser);

    let data = png::black_png(
        render_target,
        state.shared_render_data.clone(),
        &state.config_file,
    )
    .wrap_err("render black box")
    .wrap_err_png(render_target, &state.shared_render_data, &state.config_file)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .body(Body::from(Bytes::from(data)))
        .unwrap())
}
