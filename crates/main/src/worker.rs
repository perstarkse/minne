use std::sync::Arc;

use common::{
    ingress::content_processor::ContentProcessor,
    storage::{
        db::SurrealDbClient,
        types::job::{Job, JobStatus},
    },
    utils::config::get_config,
};
use futures::StreamExt;
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

    let content_processor = ContentProcessor::new(db.clone(), openai_client.clone()).await?;

    loop {
        // First, check for any unfinished jobs
        let unfinished_jobs = Job::get_unfinished_jobs(&db).await?;

        if !unfinished_jobs.is_empty() {
            info!("Found {} unfinished jobs", unfinished_jobs.len());

            for job in unfinished_jobs {
                content_processor.process_job(job).await?;
            }
        }

        // If no unfinished jobs, start listening for new ones
        info!("Listening for new jobs...");
        let mut job_stream = Job::listen_for_jobs(&db).await?;

        while let Some(notification) = job_stream.next().await {
            match notification {
                Ok(notification) => {
                    info!("Received notification: {:?}", notification);

                    match notification.action {
                        Action::Create => {
                            if let Err(e) = content_processor.process_job(notification.data).await {
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
                                        db.get_item::<Job>(&notification.data.id).await
                                    {
                                        match current_job.status {
                                            JobStatus::Error(_)
                                                if attempts
                                                    < common::storage::types::job::MAX_ATTEMPTS =>
                                            {
                                                // This is a retry after an error
                                                if let Err(e) =
                                                    content_processor.process_job(current_job).await
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
                                    if let Err(e) =
                                        content_processor.process_job(notification.data).await
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
