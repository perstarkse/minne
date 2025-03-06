use std::sync::Arc;

use common::{
    storage::{
        db::SurrealDbClient,
        types::ingestion_task::{IngestionTask, IngestionTaskStatus},
    },
    utils::config::get_config,
};
use futures::StreamExt;
use ingestion_pipeline::pipeline::IngestionPipeline;
use surrealdb::Action;
use tracing::{error, info};
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

    let openai_client = Arc::new(async_openai::Client::new());

    let ingestion_pipeline = IngestionPipeline::new(db.clone(), openai_client.clone()).await?;

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
