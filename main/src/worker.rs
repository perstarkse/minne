mod bootstrap;

use std::sync::Arc;

use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let services = bootstrap::init().await?;

    info!(
        embedding_backend = ?services.config.embedding_backend,
        "Embedding provider initialized for worker"
    );

    let ingestion_pipeline = Arc::new(IngestionPipeline::new(
        Arc::clone(&services.db),
        Arc::clone(&services.openai_client),
        services.config.clone(),
        services.reranker_pool.clone(),
        services.storage,
        Arc::clone(&services.embedding_provider),
    )?);

    run_worker_loop(services.db, ingestion_pipeline).await
}
