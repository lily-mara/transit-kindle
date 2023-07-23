use axum::{
    body::{Bytes, Full},
    extract::State,
    http::StatusCode,
    response::{Redirect, Response},
    routing::get,
    Router,
};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{api_client::Client, config::ConfigFile, render};

#[derive(Clone)]
struct AppState {
    client: Client,
    config_file: ConfigFile,
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

macro_rules! try_png {
    ($result:expr, $config_file:expr) => {
        match $result {
            Ok(x) => x,
            Err(e) => {
                let data = render::error_png($config_file, e).unwrap();
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("Content-Type", "image/png")
                    .body(Full::new(Bytes::from(data)))
                    .unwrap();
            }
        }
    };
}

async fn handle_index() -> Redirect {
    Redirect::temporary("/stops.png")
}

async fn handle_stops_png(State(state): State<AppState>) -> Response<Full<Bytes>> {
    let stop_data = try_png!(
        state.client.load_stop_data(state.config_file.clone()).await,
        &state.config_file
    );

    let data = try_png!(
        render::stops_png(stop_data, &state.config_file),
        &state.config_file
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}
