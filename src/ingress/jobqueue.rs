use chrono::Utc;
use futures::Stream;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use surrealdb::{opt::PatchOp, Error, Notification};
use tracing::{error, info};

use crate::{
    error::AppError,
    storage::{
        db::{delete_item, get_item, store_item, SurrealDbClient},
        types::{
            job::{Job, JobStatus},
            StoredObject,
        },
    },
};

use super::{content_processor::ContentProcessor, types::ingress_object::IngressObject};

pub struct JobQueue {
    pub db: Arc<SurrealDbClient>,
}

pub const MAX_ATTEMPTS: u32 = 3;

impl JobQueue {
    pub fn new(db: Arc<SurrealDbClient>) -> Self {
        Self { db }
    }

    /// Creates a new job and stores it in the database
    pub async fn enqueue(&self, content: IngressObject, user_id: String) -> Result<Job, AppError> {
        let job = Job::new(content, user_id).await;
        info!("{:?}", job);
        store_item(&self.db, job.clone()).await?;
        Ok(job)
    }

    /// Gets all jobs for a specific user
    pub async fn get_user_jobs(&self, user_id: &str) -> Result<Vec<Job>, AppError> {
        let jobs: Vec<Job> = self
            .db
            .query("SELECT * FROM job WHERE user_id = $user_id ORDER BY created_at DESC")
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;

        info!("{:?}", jobs);
        Ok(jobs)
    }

    pub async fn delete_job(&self, id: &str, user_id: &str) -> Result<(), AppError> {
        get_item::<Job>(&self.db.client, id)
            .await?
            .filter(|job| job.user_id == user_id)
            .ok_or_else(|| {
                error!("Unauthorized attempt to delete job {id} by user {user_id}");
                AppError::Auth("Not authorized to delete this job".into())
            })?;

        info!("Deleting job {id} for user {user_id}");
        delete_item::<Job>(&self.db.client, id)
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    pub async fn update_status(
        &self,
        id: &str,
        status: JobStatus,
    ) -> Result<Option<Job>, AppError> {
        let job: Option<Job> = self
            .db
            .update((Job::table_name(), id))
            .patch(PatchOp::replace("/status", status))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::sql::Datetime::default(),
            ))
            .await?;

        Ok(job)
    }

    /// Listen for new jobs
    pub async fn listen_for_jobs(
        &self,
    ) -> Result<impl Stream<Item = Result<Notification<Job>, Error>>, Error> {
        self.db.select("job").live().await
    }

    /// Get unfinished jobs, ie newly created and in progress up two times
    pub async fn get_unfinished_jobs(&self) -> Result<Vec<Job>, AppError> {
        let jobs: Vec<Job> = self
            .db
            .query(
                "SELECT * FROM type::table($table) 
             WHERE 
                status = 'Created' 
                OR (
                    status.InProgress != NONE 
                    AND status.InProgress.attempts < $max_attempts
                )
             ORDER BY created_at ASC",
            )
            .bind(("table", Job::table_name()))
            .bind(("max_attempts", MAX_ATTEMPTS))
            .await?
            .take(0)?;

        Ok(jobs)
    }

    // Method to process a single job
    pub async fn process_job(
        &self,
        job: Job,
        processor: &ContentProcessor,
    ) -> Result<(), AppError> {
        let current_attempts = match job.status {
            JobStatus::InProgress { attempts, .. } => attempts + 1,
            _ => 1,
        };

        // Update status to InProgress with attempt count
        self.update_status(
            &job.id,
            JobStatus::InProgress {
                attempts: current_attempts,
                last_attempt: Utc::now(),
            },
        )
        .await?;

        let text_content = job.content.to_text_content().await?;

        match processor.process(&text_content).await {
            Ok(_) => {
                self.update_status(&job.id, JobStatus::Completed).await?;
                Ok(())
            }
            Err(e) => {
                if current_attempts >= MAX_ATTEMPTS {
                    self.update_status(
                        &job.id,
                        JobStatus::Error(format!("Max attempts reached: {}", e)),
                    )
                    .await?;
                }
                Err(AppError::Processing(e.to_string()))
            }
        }
    }
}
