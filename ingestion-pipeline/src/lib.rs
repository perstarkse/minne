#![allow(clippy::missing_docs_in_private_items, clippy::result_large_err)]

pub mod pipeline;
pub mod utils;

use chrono::Utc;
use common::storage::{
    db::SurrealDbClient,
    types::ingestion_task::{IngestionTask, DEFAULT_LEASE_SECS},
};
pub use pipeline::{IngestionConfig, IngestionPipeline, IngestionTuning};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};
use uuid::Uuid;

pub async fn run_worker_loop(
    db: Arc<SurrealDbClient>,
    ingestion_pipeline: Arc<IngestionPipeline>,
) -> Result<(), Box<dyn std::error::Error>> {
    let worker_id = format!("ingestion-worker-{}", Uuid::new_v4());
    let lease_duration = Duration::from_secs(DEFAULT_LEASE_SECS as u64);
    let idle_backoff = Duration::from_millis(500);

    loop {
        match IngestionTask::claim_next_ready(&db, &worker_id, Utc::now(), lease_duration).await {
            Ok(Some(task)) => {
                let task_id = task.id.clone();
                info!(
                    %worker_id,
                    %task_id,
                    attempt = task.attempts,
                    "claimed ingestion task"
                );
                if let Err(err) = ingestion_pipeline.process_task(task).await {
                    error!(%worker_id, %task_id, error = %err, "ingestion task failed");
                }
            }
            Ok(None) => {
                sleep(idle_backoff).await;
            }
            Err(err) => {
                error!(%worker_id, error = %err, "failed to claim ingestion task");
                warn!("Backing off for 1s after claim error");
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}
