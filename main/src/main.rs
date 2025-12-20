use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::{
    storage::{
        db::SurrealDbClient, indexes::ensure_runtime_indexes, store::StorageManager,
        types::{
            knowledge_entity::KnowledgeEntity, system_settings::SystemSettings,
            text_chunk::TextChunk,
        },
    },
    utils::{config::get_config, embedding::EmbeddingProvider},
};
use html_router::{html_routes, html_state::HtmlState};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use retrieval_pipeline::reranking::RerankerPool;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use tokio::task::LocalSet;

#[tokio::main]
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

    // Create embedding provider based on config before syncing settings.
    let embedding_provider =
        Arc::new(EmbeddingProvider::from_config(&config, Some(openai_client.clone())).await?);
    info!(
        embedding_backend = ?config.embedding_backend,
        embedding_dimension = embedding_provider.dimension(),
        "Embedding provider initialized"
    );

    // Sync SystemSettings with provider's dimensions/model/backend
    let (settings, dimensions_changed) =
        SystemSettings::sync_from_embedding_provider(&db, &embedding_provider).await?;

    // Now ensure runtime indexes with the correct (synced) dimensions
    ensure_runtime_indexes(&db, settings.embedding_dimensions as usize).await?;

    // If dimensions changed, re-embed existing data to keep queries working.
    if dimensions_changed {
        warn!(
            new_dimensions = settings.embedding_dimensions,
            "Embedding configuration changed; re-embedding existing data"
        );

        // Re-embed text chunks
        info!("Re-embedding TextChunks");
        if let Err(e) = TextChunk::update_all_embeddings_with_provider(
            &db,
            &embedding_provider,
        )
        .await
        {
            error!("Failed to re-embed TextChunks: {}. Search results may be stale.", e);
        }

        // Re-embed knowledge entities
        info!("Re-embedding KnowledgeEntities");
        if let Err(e) = KnowledgeEntity::update_all_embeddings_with_provider(
            &db,
            &embedding_provider,
        )
        .await
        {
            error!(
                "Failed to re-embed KnowledgeEntities: {}. Search results may be stale.",
                e
            );
        }

        info!("Re-embedding complete.");
    }

    let reranker_pool = RerankerPool::maybe_from_config(&config)?;

    // Create global storage manager
    let storage = StorageManager::new(&config).await?;

    let html_state = HtmlState::new_with_resources(
        db,
        openai_client,
        session_store,
        storage.clone(),
        config.clone(),
        reranker_pool.clone(),
        embedding_provider.clone(),
    )
    .await?;

    let api_state = ApiState::new(&config, storage.clone()).await?;

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
        let openai_client = Arc::new(async_openai::Client::with_config(
            async_openai::config::OpenAIConfig::new()
                .with_api_key(&config.openai_api_key)
                .with_api_base(&config.openai_base_url),
        ));

        // Create embedding provider based on config
        let embedding_provider = Arc::new(
            EmbeddingProvider::from_config(&config, Some(openai_client.clone()))
                .await
                .expect("failed to create embedding provider"),
        );
        let ingestion_pipeline = Arc::new(
            IngestionPipeline::new(
                worker_db.clone(),
                openai_client.clone(),
                config.clone(),
                reranker_pool.clone(),
                storage.clone(),
                embedding_provider,
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, http::StatusCode, Router};
    use common::storage::store::StorageManager;
    use common::utils::config::{AppConfig, PdfIngestMode, StorageKind};
    use std::{path::Path, sync::Arc};
    use tower::ServiceExt;
    use uuid::Uuid;

    fn smoke_test_config(namespace: &str, database: &str, data_dir: &Path) -> AppConfig {
        AppConfig {
            openai_api_key: "test-key".into(),
            surrealdb_address: "mem://".into(),
            surrealdb_username: "root".into(),
            surrealdb_password: "root".into(),
            surrealdb_namespace: namespace.into(),
            surrealdb_database: database.into(),
            data_dir: data_dir.to_string_lossy().into_owned(),
            http_port: 0,
            openai_base_url: "https://example.com".into(),
            storage: StorageKind::Local,
            pdf_ingest_mode: PdfIngestMode::LlmFirst,
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn smoke_startup_with_in_memory_surrealdb() {
        let namespace = "test_ns";
        let database = format!("test_db_{}", Uuid::new_v4());
        let data_dir = std::env::temp_dir().join(format!("minne_smoke_{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&data_dir)
            .await
            .expect("failed to create temp data directory");

        let config = smoke_test_config(namespace, &database, &data_dir);
        let db = Arc::new(
            SurrealDbClient::memory(namespace, &database)
                .await
                .expect("failed to start in-memory surrealdb"),
        );
        db.apply_migrations()
            .await
            .expect("failed to apply migrations");

        let session_store = Arc::new(db.create_session_store().await.expect("session store"));
        let openai_client = Arc::new(async_openai::Client::with_config(
            async_openai::config::OpenAIConfig::new()
                .with_api_key(&config.openai_api_key)
                .with_api_base(&config.openai_base_url),
        ));

        let storage = StorageManager::new(&config)
            .await
            .expect("failed to build storage manager");

        // Use hashed embeddings for tests to avoid external dependencies
        let embedding_provider = Arc::new(
            common::utils::embedding::EmbeddingProvider::new_hashed(384)
                .expect("failed to create hashed embedding provider"),
        );

        let html_state = HtmlState::new_with_resources(
            db.clone(),
            openai_client,
            session_store,
            storage.clone(),
            config.clone(),
            None,
            embedding_provider,
        )
        .await
        .expect("failed to build html state");

        let api_state = ApiState {
            db: db.clone(),
            config: config.clone(),
            storage,
        };

        let app = Router::new()
            .nest("/api/v1", api_routes_v1(&api_state))
            .merge(html_routes(&html_state))
            .with_state(AppState {
                api_state,
                html_state,
            });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/live")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK);

        let ready_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ready")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("ready response");
        assert_eq!(ready_response.status(), StatusCode::OK);

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }
}
