use futures::Stream;
use surrealdb::{opt::PatchOp, Notification};
use uuid::Uuid;

use crate::{
    error::AppError, ingress::ingress_object::IngressObject, storage::db::SurrealDbClient,
    stored_object,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    Created,
    InProgress {
        attempts: u32,
        last_attempt: DateTime<Utc>,
    },
    Completed,
    Error(String),
    Cancelled,
}

stored_object!(Job, "job", {
    content: IngressObject,
    status: JobStatus,
    user_id: String
});

pub const MAX_ATTEMPTS: u32 = 3;

impl Job {
    pub async fn new(content: IngressObject, user_id: String) -> Self {
        let now = Utc::now();

        Self {
            id: Uuid::new_v4().to_string(),
            content,
            status: JobStatus::Created,
            created_at: now,
            updated_at: now,
            user_id,
        }
    }

    /// Creates a new job and stores it in the database
    pub async fn create_and_add_to_db(
        content: IngressObject,
        user_id: String,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let job = Self::new(content, user_id).await;

        db.store_item(job).await?;

        Ok(())
    }

    // Update job status
    pub async fn update_status(
        id: &str,
        status: JobStatus,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let _job: Option<Job> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/status", status))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::sql::Datetime::default(),
            ))
            .await?;

        Ok(())
    }

    /// Listen for new jobs
    pub async fn listen_for_jobs(
        db: &SurrealDbClient,
    ) -> Result<impl Stream<Item = Result<Notification<Job>, surrealdb::Error>>, surrealdb::Error>
    {
        db.listen::<Job>().await
    }

    /// Get all unfinished jobs, ie newly created and in progress up two times
    pub async fn get_unfinished_jobs(db: &SurrealDbClient) -> Result<Vec<Job>, AppError> {
        let jobs: Vec<Job> = db
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
            .bind(("table", Self::table_name()))
            .bind(("max_attempts", MAX_ATTEMPTS))
            .await?
            .take(0)?;

        Ok(jobs)
    }
}
