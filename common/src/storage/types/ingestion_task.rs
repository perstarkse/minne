use futures::Stream;
use surrealdb::{opt::PatchOp, Notification};
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::ingestion_payload::IngestionPayload;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // Helper function to create a test ingestion payload
    fn create_test_payload(user_id: &str) -> IngestionPayload {
        IngestionPayload::Text {
            text: "Test content".to_string(),
            instructions: "Test instructions".to_string(),
            category: "Test category".to_string(),
            user_id: user_id.to_string(),
        }
    }

    #[tokio::test]
    async fn test_new_ingestion_task() {
        let user_id = "user123";
        let payload = create_test_payload(user_id);

        let task = IngestionTask::new(payload.clone(), user_id.to_string()).await;

        // Verify task properties
        assert_eq!(task.user_id, user_id);
        assert_eq!(task.content, payload);
        assert!(matches!(task.status, IngestionTaskStatus::Created));
        assert!(!task.id.is_empty());
    }

    #[tokio::test]
    async fn test_create_and_add_to_db() {
        // Setup in-memory database
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let user_id = "user123";
        let payload = create_test_payload(user_id);

        // Create and store task
        IngestionTask::create_and_add_to_db(payload.clone(), user_id.to_string(), &db)
            .await
            .expect("Failed to create and add task to db");

        // Query to verify task was stored
        let query = format!(
            "SELECT * FROM {} WHERE user_id = '{}'",
            IngestionTask::table_name(),
            user_id
        );
        let mut result = db.query(query).await.expect("Query failed");
        let tasks: Vec<IngestionTask> = result.take(0).unwrap_or_default();

        // Verify task is in the database
        assert!(!tasks.is_empty(), "Task should exist in the database");
        let stored_task = &tasks[0];
        assert_eq!(stored_task.user_id, user_id);
        assert!(matches!(stored_task.status, IngestionTaskStatus::Created));
    }

    #[tokio::test]
    async fn test_update_status() {
        // Setup in-memory database
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let user_id = "user123";
        let payload = create_test_payload(user_id);

        // Create task manually
        let task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        let task_id = task.id.clone();

        // Store task
        db.store_item(task).await.expect("Failed to store task");

        // Update status to InProgress
        let now = Utc::now();
        let new_status = IngestionTaskStatus::InProgress {
            attempts: 1,
            last_attempt: now,
        };

        IngestionTask::update_status(&task_id, new_status.clone(), &db)
            .await
            .expect("Failed to update status");

        // Verify status updated
        let updated_task: Option<IngestionTask> = db
            .get_item::<IngestionTask>(&task_id)
            .await
            .expect("Failed to get updated task");

        assert!(updated_task.is_some());
        let updated_task = updated_task.unwrap();

        match updated_task.status {
            IngestionTaskStatus::InProgress { attempts, .. } => {
                assert_eq!(attempts, 1);
            }
            _ => panic!("Expected InProgress status"),
        }
    }

    #[tokio::test]
    async fn test_get_unfinished_tasks() {
        // Setup in-memory database
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let user_id = "user123";
        let payload = create_test_payload(user_id);

        // Create tasks with different statuses
        let created_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;

        let mut in_progress_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        in_progress_task.status = IngestionTaskStatus::InProgress {
            attempts: 1,
            last_attempt: Utc::now(),
        };

        let mut max_attempts_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        max_attempts_task.status = IngestionTaskStatus::InProgress {
            attempts: MAX_ATTEMPTS,
            last_attempt: Utc::now(),
        };

        let mut completed_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        completed_task.status = IngestionTaskStatus::Completed;

        let mut error_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        error_task.status = IngestionTaskStatus::Error("Test error".to_string());

        // Store all tasks
        db.store_item(created_task)
            .await
            .expect("Failed to store created task");
        db.store_item(in_progress_task)
            .await
            .expect("Failed to store in-progress task");
        db.store_item(max_attempts_task)
            .await
            .expect("Failed to store max-attempts task");
        db.store_item(completed_task)
            .await
            .expect("Failed to store completed task");
        db.store_item(error_task)
            .await
            .expect("Failed to store error task");

        // Get unfinished tasks
        let unfinished_tasks = IngestionTask::get_unfinished_tasks(&db)
            .await
            .expect("Failed to get unfinished tasks");

        // Verify only Created and InProgress with attempts < MAX_ATTEMPTS are returned
        assert_eq!(unfinished_tasks.len(), 2);

        let statuses: Vec<_> = unfinished_tasks
            .iter()
            .map(|task| match &task.status {
                IngestionTaskStatus::Created => "Created",
                IngestionTaskStatus::InProgress { attempts, .. } => {
                    if *attempts < MAX_ATTEMPTS {
                        "InProgress<MAX"
                    } else {
                        "InProgress>=MAX"
                    }
                }
                IngestionTaskStatus::Completed => "Completed",
                IngestionTaskStatus::Error(_) => "Error",
                IngestionTaskStatus::Cancelled => "Cancelled",
            })
            .collect();

        assert!(statuses.contains(&"Created"));
        assert!(statuses.contains(&"InProgress<MAX"));
        assert!(!statuses.contains(&"InProgress>=MAX"));
        assert!(!statuses.contains(&"Completed"));
        assert!(!statuses.contains(&"Error"));
        assert!(!statuses.contains(&"Cancelled"));
    }
}
