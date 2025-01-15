use std::sync::Arc;

use futures::StreamExt;
use surrealdb::Action;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{
    ingress::{
        content_processor::ContentProcessor,
        jobqueue::{JobQueue, MAX_ATTEMPTS},
    },
    storage::{
        db::{get_item, SurrealDbClient},
        types::job::{Job, JobStatus},
    },
    utils::config::get_config,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;

    let surreal_db_client = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    );

    let job_queue = JobQueue::new(surreal_db_client.clone());

    let content_processor = ContentProcessor::new(surreal_db_client).await?;

    loop {
        // First, check for any unfinished jobs
        let unfinished_jobs = job_queue.get_unfinished_jobs().await?;

        if !unfinished_jobs.is_empty() {
            info!("Found {} unfinished jobs", unfinished_jobs.len());

            for job in unfinished_jobs {
                job_queue.process_job(job, &content_processor).await?;
            }
        }

        // If no unfinished jobs, start listening for new ones
        info!("Listening for new jobs...");
        let mut job_stream = job_queue.listen_for_jobs().await?;

        while let Some(notification) = job_stream.next().await {
            match notification {
                Ok(notification) => {
                    info!("Received notification: {:?}", notification);

                    match notification.action {
                        Action::Create => {
                            if let Err(e) = job_queue
                                .process_job(notification.data, &content_processor)
                                .await
                            {
                                error!("Error processing job: {}", e);
                            }
                        }
                        Action::Update => {
                            match notification.data.status {
                                JobStatus::Completed
                                | JobStatus::Error(_)
                                | JobStatus::Cancelled => {
                                    info!(
                                        "Skipping already completed/error/cancelled job: {}",
                                        notification.data.id
                                    );
                                    continue;
                                }
                                JobStatus::InProgress { attempts, .. } => {
                                    // Only process if this is a retry after an error, not our own update
                                    if let Ok(Some(current_job)) =
                                        get_item::<Job>(&job_queue.db.client, &notification.data.id)
                                            .await
                                    {
                                        match current_job.status {
                                            JobStatus::Error(_) if attempts < MAX_ATTEMPTS => {
                                                // This is a retry after an error
                                                if let Err(e) = job_queue
                                                    .process_job(current_job, &content_processor)
                                                    .await
                                                {
                                                    error!("Error processing job retry: {}", e);
                                                }
                                            }
                                            _ => {
                                                info!(
                                                    "Skipping in-progress update for job: {}",
                                                    notification.data.id
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                }
                                JobStatus::Created => {
                                    // Shouldn't happen with Update action, but process if it does
                                    if let Err(e) = job_queue
                                        .process_job(notification.data, &content_processor)
                                        .await
                                    {
                                        error!("Error processing job: {}", e);
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
        error!("Job stream ended unexpectedly, reconnecting...");
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
