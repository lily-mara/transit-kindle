use axum::{
    body::{Bytes, Full},
    extract::State,
    headers::UserAgent,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router, TypedHeader,
};
use eyre::Context;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    api_client::Client,
    config::ConfigFile,
    render::{self, RenderTarget},
};

#[derive(Clone)]
struct AppState {
    client: Client,
    config_file: ConfigFile,
}

struct ErrorPng {
    data: Vec<u8>,
}

trait WrapErrPng<T> {
    fn wrap_err_png(
        self,
        render_target: RenderTarget,
        config_file: &ConfigFile,
    ) -> Result<T, ErrorPng>;
}

impl<T> WrapErrPng<T> for eyre::Result<T> {
    fn wrap_err_png(
        self,
        render_target: RenderTarget,
        config_file: &ConfigFile,
    ) -> Result<T, ErrorPng> {
        match self {
            Ok(x) => Ok(x),
            Err(error) => Err(ErrorPng {
                data: render::error_png(render_target, config_file, error).unwrap(),
            }),
        }
    }
}

impl IntoResponse for ErrorPng {
    fn into_response(self) -> Response {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("Content-Type", "image/png")
            .body(Full::new(Bytes::from(self.data)))
            .unwrap()
            .into_response()
    }
}

pub async fn serve(client: Client, config_file: ConfigFile) -> eyre::Result<()> {
    let app = Router::new()
        .route("/stops.png", get(handle_stops_png))
        .route("/", get(handle_index))
        .with_state(AppState {
            client,
            config_file,
        })
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));

    info!(port = 3001, "listening!");

    axum::Server::bind(&"0.0.0.0:3001".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn handle_index() -> Redirect {
    Redirect::temporary("/stops.png")
}

fn render_target(maybe_user_agent: Option<TypedHeader<UserAgent>>) -> RenderTarget {
    if maybe_user_agent.is_some() {
        RenderTarget::Other
    } else {
        RenderTarget::Kindle
    }
}

async fn handle_stops_png(
    State(state): State<AppState>,
    maybe_user_agent: Option<TypedHeader<UserAgent>>,
) -> Result<Response<Full<Bytes>>, ErrorPng> {
    let render_target = render_target(maybe_user_agent);

    let stop_data = state
        .client
        .load_stop_data(state.config_file.clone())
        .await
        .wrap_err("load stop data")
        .wrap_err_png(render_target, &state.config_file)?;

    let data = render::stops_png(render_target, stop_data, &state.config_file)
        .wrap_err("render schedule")
        .wrap_err_png(render_target, &state.config_file)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .body(Full::new(Bytes::from(data)))
        .unwrap())
}
