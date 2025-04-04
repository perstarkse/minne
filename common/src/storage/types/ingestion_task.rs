use futures::Stream;
use surrealdb::{opt::PatchOp, Notification};
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::ingestion_payload::IngestionPayload;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IngestionTaskStatus {
    Created,
    InProgress {
        attempts: u32,
        last_attempt: DateTime<Utc>,
    },
    Completed,
    Error(String),
    Cancelled,
}

stored_object!(IngestionTask, "job", {
    content: IngestionPayload,
    status: IngestionTaskStatus,
    user_id: String
});

pub const MAX_ATTEMPTS: u32 = 3;

impl IngestionTask {
    pub async fn new(content: IngestionPayload, user_id: String) -> Self {
        let now = Utc::now();

        Self {
            id: Uuid::new_v4().to_string(),
            content,
            status: IngestionTaskStatus::Created,
            created_at: now,
            updated_at: now,
            user_id,
        }
    }

    /// Creates a new job and stores it in the database
    pub async fn create_and_add_to_db(
        content: IngestionPayload,
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
        status: IngestionTaskStatus,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let _job: Option<Self> = db
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
    pub async fn listen_for_tasks(
        db: &SurrealDbClient,
    ) -> Result<impl Stream<Item = Result<Notification<Self>, surrealdb::Error>>, surrealdb::Error>
    {
        db.listen::<Self>().await
    }

    /// Get all unfinished tasks, ie newly created and in progress up two times
    pub async fn get_unfinished_tasks(db: &SurrealDbClient) -> Result<Vec<Self>, AppError> {
        let jobs: Vec<Self> = db
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
