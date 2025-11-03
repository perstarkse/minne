use std::sync::Arc;

use common::{
    storage::db::SurrealDbClient, storage::store::StorageManager, utils::config::get_config,
};
use composite_retrieval::reranking::RerankerPool;
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;

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

    let openai_client = Arc::new(async_openai::Client::with_config(
        async_openai::config::OpenAIConfig::new()
            .with_api_key(&config.openai_api_key)
            .with_api_base(&config.openai_base_url),
    ));

    let reranker_pool = RerankerPool::maybe_from_config(&config)?;

    // Create global storage manager
    let storage = StorageManager::new(&config).await?;

    let ingestion_pipeline = Arc::new(
        IngestionPipeline::new(
            db.clone(),
            openai_client.clone(),
            config,
            reranker_pool,
            storage,
        )
        .await?,
    );

    run_worker_loop(db, ingestion_pipeline).await
}
