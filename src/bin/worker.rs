use std::sync::Arc;

use futures::StreamExt;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{
    ingress::{content_processor::ContentProcessor, jobqueue::JobQueue},
    storage::db::SurrealDbClient,
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

    let job_queue = JobQueue::new(Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    ));

    let content_processor = ContentProcessor::new(&config).await?;

    loop {
        // First, check for any unfinished jobs
        let unfinished_jobs = job_queue.get_unfinished_jobs().await?;

        if !unfinished_jobs.is_empty() {
            info!("Found {} unfinished jobs", unfinished_jobs.len());

            for job in unfinished_jobs {
                if let Err(e) = job_queue.process_job(job.clone(), &content_processor).await {
                    error!("Error processing job {}: {}", job.id, e);
                }
            }
        }

        // If no unfinished jobs, start listening for new ones
        info!("Listening for new jobs...");
        let mut job_stream = job_queue.listen_for_jobs().await?;

        while let Some(notification) = job_stream.next().await {
            match notification {
                Ok(notification) => {
                    info!("Received new job: {}", notification.data.id);
                    if let Err(e) = job_queue
                        .process_job(notification.data, &content_processor)
                        .await
                    {
                        error!("Error processing job: {}", e);
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
