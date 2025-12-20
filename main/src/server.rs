use std::sync::Arc;

use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::{
    storage::{db::SurrealDbClient, store::StorageManager, types::system_settings::SystemSettings},
    utils::{config::get_config, embedding::EmbeddingProvider},
};
use html_router::{html_routes, html_state::HtmlState};
use retrieval_pipeline::reranking::RerankerPool;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
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
        async_openai::config::OpenAIConfig::new()
            .with_api_key(&config.openai_api_key)
            .with_api_base(&config.openai_base_url),
    ));

    let reranker_pool = RerankerPool::maybe_from_config(&config)?;

    // Create global storage manager
    let storage = StorageManager::new(&config).await?;

    // Create embedding provider based on config
    let embedding_provider = Arc::new(
        EmbeddingProvider::from_config(&config, Some(openai_client.clone())).await?,
    );
    info!(
        embedding_backend = ?config.embedding_backend,
        embedding_dimension = embedding_provider.dimension(),
        "Embedding provider initialized"
    );

    // Sync SystemSettings with provider's dimensions/backend for visibility
    let (_settings, _dimensions_changed) =
        SystemSettings::sync_from_embedding_provider(&db, &embedding_provider).await?;

    let html_state = HtmlState::new_with_resources(
        db,
        openai_client,
        session_store,
        storage.clone(),
        config.clone(),
        reranker_pool,
        embedding_provider,
    )
    .await?;

    let api_state = ApiState::new(&config, storage).await?;

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
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: ApiState,
    html_state: HtmlState,
}
