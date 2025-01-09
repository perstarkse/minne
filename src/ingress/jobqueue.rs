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
        db::{store_item, SurrealDbClient},
        types::{
            job::{Job, JobStatus},
            StoredObject,
        },
    },
};

use super::{content_processor::ContentProcessor, types::ingress_object::IngressObject};

pub struct JobQueue {
    db: Arc<SurrealDbClient>,
}

const MAX_ATTEMPTS: u32 = 3;

impl JobQueue {
    pub fn new(db: Arc<SurrealDbClient>) -> Self {
        Self { db }
    }

    /// Creates a new job and stores it in the database
    pub async fn enqueue(&self, content: IngressObject, user_id: String) -> Result<Job, AppError> {
        let job = Job::new(content, user_id).await;
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

        Ok(jobs)
    }

    pub async fn delete_job(&self, id: &str, user_id: &str) -> Result<(), AppError> {
        // First, validate that the job exists and belongs to the user
        let job: Option<Job> = self
            .db
            .query("SELECT * FROM job WHERE id = $id AND user_id = $user_id")
            .bind(("id", id.to_string()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;

        // If no job is found or it doesn't belong to the user, return Unauthorized
        if job.is_none() {
            error!("Unauthorized attempt to delete job {id} by user {user_id}");
            return Err(AppError::Auth("Not authorized to delete this job".into()));
        }

        info!("Deleting job {id} for user {user_id}");

        // If validation passes, delete the job
        let _deleted: Option<Job> = self
            .db
            .delete((Job::table_name(), id))
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    /// Update status for job
    pub async fn update_status(
        &self,
        id: &str,
        status: JobStatus,
    ) -> Result<Option<Job>, AppError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string();

        let status_value =
            serde_json::to_value(status).map_err(|e| AppError::LLMParsing(e.to_string()))?;

        let job: Option<Job> = self
            .db
            .update((Job::table_name(), id))
            .patch(PatchOp::replace("/status", status_value))
            .patch(PatchOp::replace("/updated_at", now))
            .await?;

        Ok(job)
    }

    /// Listen for new jobs
    pub async fn listen_for_jobs(
        &self,
    ) -> Result<impl Stream<Item = Result<Notification<Job>, Error>>, Error> {
        self.db.select("job").live().await
    }

    pub async fn get_unfinished_jobs(&self) -> Result<Vec<Job>, AppError> {
        let jobs: Vec<Job> = self
            .db
            .query(
                "SELECT * FROM job WHERE status.Created = true OR (status.InProgress.attempts < $max_attempts) ORDER BY created_at ASC")
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string();

        let current_attempts = match job.status {
            JobStatus::InProgress { attempts, .. } => attempts + 1,
            _ => 1,
        };

        // Update status to InProgress with attempt count
        self.update_status(
            &job.id,
            JobStatus::InProgress {
                attempts: current_attempts,
                last_attempt: now.clone(),
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
