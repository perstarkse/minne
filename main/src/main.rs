use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::{storage::db::SurrealDbClient, utils::config::get_config};
use html_router::{html_routes, html_state::HtmlState};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use tokio::task::LocalSet;

#[tokio::main]
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
    let db = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    );

    // Ensure db is initialized
    db.apply_migrations().await?;

    let session_store = Arc::new(db.create_session_store().await?);
    let openai_client = Arc::new(async_openai::Client::with_config(
        async_openai::config::OpenAIConfig::new().with_api_key(&config.openai_api_key),
    ));

    let html_state =
        HtmlState::new_with_resources(db, openai_client, session_store, config.clone())?;

    let api_state = ApiState {
        db: html_state.db.clone(),
        config: config.clone(),
    };

    // Create Axum router
    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&api_state))
        .merge(html_routes(&html_state))
        .with_state(AppState {
            api_state,
            html_state,
        });

    info!("Starting server listening on 0.0.0.0:{}", config.http_port);
    let serve_address = format!("0.0.0.0:{}", config.http_port);
    let listener = tokio::net::TcpListener::bind(serve_address).await?;

    // Start the server in a separate OS thread with its own runtime
    let server_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if let Err(e) = axum::serve(listener, app).await {
                error!("Server error: {}", e);
            }
        });
    });

    // Create a LocalSet for the worker
    let local = LocalSet::new();

    // Use a clone of the config for the worker
    let worker_config = config.clone();

    // Run the worker in the local set
    local.spawn_local(async move {
        // Create worker db connection
        let worker_db = Arc::new(
            SurrealDbClient::new(
                &worker_config.surrealdb_address,
                &worker_config.surrealdb_username,
                &worker_config.surrealdb_password,
                &worker_config.surrealdb_namespace,
                &worker_config.surrealdb_database,
            )
            .await
            .unwrap(),
        );

        // Initialize worker components
        let openai_client = Arc::new(async_openai::Client::new());
        let ingestion_pipeline = Arc::new(
            IngestionPipeline::new(worker_db.clone(), openai_client.clone(), config.clone())
                .await
                .unwrap(),
        );

        info!("Starting worker process");
        if let Err(e) = run_worker_loop(worker_db, ingestion_pipeline).await {
            error!("Worker process error: {}", e);
        }
    });

    // Run the local set on the main thread
    local.await;

    // Wait for the server thread to finish (this likely won't be reached)
    if let Err(e) = server_handle.join() {
        error!("Server thread panicked: {:?}", e);
    }

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: ApiState,
    html_state: HtmlState,
}
