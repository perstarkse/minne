mod bootstrap;

use axum::extract::FromRef;
use bootstrap::{
    init, prepare_embedding_runtime,
    wiring::{build_api_state, build_html_state, minne_routes},
};
use tracing::info;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let services = init().await?;
    prepare_embedding_runtime(&services).await?;

    let html_state = build_html_state(&services).await?;
    let api_state = build_api_state(&services);

    let app = minne_routes(&api_state, &html_state).with_state(AppState {
        api_state,
        html_state,
    });

    info!(
        "Starting server listening on 0.0.0.0:{}",
        services.config.http_port
    );
    let serve_address = format!("0.0.0.0:{}", services.config.http_port);
    let listener = tokio::net::TcpListener::bind(serve_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: api_router::api_state::ApiState,
    html_state: html_router::html_state::HtmlState,
}
