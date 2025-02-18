use axum::Router;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{
    server::{
        routes::{api_routes_v1, html_routes},
        AppState,
    },
    utils::config::get_config,
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;

    let app_state = AppState::new(&config).await?;

    // Create Axum router
    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&app_state))
        .nest("/", html_routes(&app_state))
        .with_state(app_state);

    info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
