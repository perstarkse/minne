use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::utils::config::get_config;
use html_router::{html_routes, html_state::HtmlState};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    // Get config
    let config = get_config()?;

    // Set up router states
    let html_state = HtmlState::new(&config).await?;
    let api_state = ApiState {
        db: html_state.db.clone(),
    };

    // Create Axum router
    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&api_state))
        .nest("/", html_routes(&html_state))
        .with_state(AppState {
            api_state,
            html_state,
        });

    info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: ApiState,
    html_state: HtmlState,
}
