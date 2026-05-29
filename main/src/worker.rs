mod bootstrap;

use std::sync::Arc;

use bootstrap::{init, prepare_embedding_runtime};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let services = init().await?;
    prepare_embedding_runtime(&services).await?;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use common::storage::types::ingestion_task::{IngestionTask, DEFAULT_LEASE_SECS};
    use ingestion_pipeline::pipeline::IngestionPipeline;

    use crate::bootstrap::tests::init_smoke_services;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_smoke_initializes_and_claims_idle() -> anyhow::Result<()> {
        let (services, data_dir) = init_smoke_services().await?;

        let pipeline = IngestionPipeline::new(
            Arc::clone(&services.db),
            Arc::clone(&services.openai_client),
            services.config.clone(),
            services.reranker_pool.clone(),
            services.storage,
            Arc::clone(&services.embedding_provider),
        )?;

        let worker_id = "worker-smoke";
        let claimed = IngestionTask::claim_next_ready(
            &services.db,
            worker_id,
            Utc::now(),
            Duration::from_secs(DEFAULT_LEASE_SECS as u64),
        )
        .await?;
        assert!(
            claimed.is_none(),
            "worker smoke test should find no pending tasks"
        );

        let db = Arc::clone(&services.db);
        let pipeline = Arc::new(pipeline);
        let worker = tokio::spawn(async move {
            ingestion_pipeline::run_worker_loop(db, pipeline).await
        });

        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(
            !worker.is_finished(),
            "worker loop should keep running while idle"
        );
        worker.abort();

        tokio::fs::remove_dir_all(&data_dir).await.ok();
        Ok(())
    }
}
