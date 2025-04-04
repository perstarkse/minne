pub mod enricher;
pub mod pipeline;
pub mod types;
pub mod utils;

use common::storage::{
    db::SurrealDbClient,
    types::ingestion_task::{IngestionTask, IngestionTaskStatus},
};
use futures::StreamExt;
use pipeline::IngestionPipeline;
use std::sync::Arc;
use surrealdb::Action;
use tracing::{error, info};

pub async fn run_worker_loop(
    db: Arc<SurrealDbClient>,
    ingestion_pipeline: Arc<IngestionPipeline>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        // First, check for any unfinished tasks
        let unfinished_tasks = IngestionTask::get_unfinished_tasks(&db).await?;
        if !unfinished_tasks.is_empty() {
            info!("Found {} unfinished jobs", unfinished_tasks.len());
            for task in unfinished_tasks {
                ingestion_pipeline.process_task(task).await?;
            }
        }

        // If no unfinished jobs, start listening for new ones
        info!("Listening for new jobs...");
        let mut job_stream = IngestionTask::listen_for_tasks(&db).await?;
        while let Some(notification) = job_stream.next().await {
            match notification {
                Ok(notification) => {
                    info!("Received notification: {:?}", notification);
                    match notification.action {
                        Action::Create => {
                            if let Err(e) = ingestion_pipeline.process_task(notification.data).await
                            {
                                error!("Error processing task: {}", e);
                            }
                        }
                        Action::Update => {
                            match notification.data.status {
                                IngestionTaskStatus::Completed
                                | IngestionTaskStatus::Error(_)
                                | IngestionTaskStatus::Cancelled => {
                                    info!(
                                        "Skipping already completed/error/cancelled task: {}",
                                        notification.data.id
                                    );
                                    continue;
                                }
                                IngestionTaskStatus::InProgress { attempts, .. } => {
                                    // Only process if this is a retry after an error, not our own update
                                    if let Ok(Some(current_task)) =
                                        db.get_item::<IngestionTask>(&notification.data.id).await
                                    {
                                        match current_task.status {
                                              IngestionTaskStatus::Error(_)
                                                  if attempts
                                                      < common::storage::types::ingestion_task::MAX_ATTEMPTS =>
                                              {
                                                  // This is a retry after an error
                                                  if let Err(e) =
                                                      ingestion_pipeline.process_task(current_task).await
                                                  {
                                                      error!("Error processing task retry: {}", e);
                                                  }
                                              }
                                              _ => {
                                                  info!(
                                                      "Skipping in-progress update for task: {}",
                                                      notification.data.id
                                                  );
                                                  continue;
                                              }
                                          }
                                    }
                                }
                                IngestionTaskStatus::Created => {
                                    // Shouldn't happen with Update action, but process if it does
                                    if let Err(e) =
                                        ingestion_pipeline.process_task(notification.data).await
                                    {
                                        error!("Error processing task: {}", e);
                                    }
                                }
                            }
                        }
                        _ => {} // Ignore other actions
                    }
                }
                Err(e) => error!("Error in job notification: {}", e),
            }
        }

        // If we reach here, the stream has ended (connection lost?)
        error!("Database stream ended unexpectedly, reconnecting...");
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
