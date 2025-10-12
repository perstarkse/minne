use std::time::Duration;

use chrono::Duration as ChronoDuration;
use state_machines::state_machine;
use surrealdb::sql::Datetime as SurrealDatetime;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::ingestion_payload::IngestionPayload;

pub const MAX_ATTEMPTS: u32 = 3;
pub const DEFAULT_LEASE_SECS: i64 = 300;
pub const DEFAULT_PRIORITY: i32 = 0;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum TaskState {
    #[serde(rename = "Pending")]
    #[default]
    Pending,
    #[serde(rename = "Reserved")]
    Reserved,
    #[serde(rename = "Processing")]
    Processing,
    #[serde(rename = "Succeeded")]
    Succeeded,
    #[serde(rename = "Failed")]
    Failed,
    #[serde(rename = "Cancelled")]
    Cancelled,
    #[serde(rename = "DeadLetter")]
    DeadLetter,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Pending => "Pending",
            TaskState::Reserved => "Reserved",
            TaskState::Processing => "Processing",
            TaskState::Succeeded => "Succeeded",
            TaskState::Failed => "Failed",
            TaskState::Cancelled => "Cancelled",
            TaskState::DeadLetter => "DeadLetter",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Succeeded | TaskState::Cancelled | TaskState::DeadLetter
        )
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            TaskState::Pending => "Pending",
            TaskState::Reserved => "Reserved",
            TaskState::Processing => "Processing",
            TaskState::Succeeded => "Completed",
            TaskState::Failed => "Retrying",
            TaskState::Cancelled => "Cancelled",
            TaskState::DeadLetter => "Dead Letter",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct TaskErrorInfo {
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
enum TaskTransition {
    Reserve,
    StartProcessing,
    Succeed,
    Fail,
    Cancel,
    DeadLetter,
    Release,
}

impl TaskTransition {
    fn as_str(&self) -> &'static str {
        match self {
            TaskTransition::Reserve => "reserve",
            TaskTransition::StartProcessing => "start_processing",
            TaskTransition::Succeed => "succeed",
            TaskTransition::Fail => "fail",
            TaskTransition::Cancel => "cancel",
            TaskTransition::DeadLetter => "deadletter",
            TaskTransition::Release => "release",
        }
    }
}

mod lifecycle {
    use super::state_machine;

    state_machine! {
        name: TaskLifecycleMachine,
        initial: Pending,
        states: [Pending, Reserved, Processing, Succeeded, Failed, Cancelled, DeadLetter],
        events {
            reserve {
                transition: { from: Pending, to: Reserved }
                transition: { from: Failed, to: Reserved }
            }
            start_processing {
                transition: { from: Reserved, to: Processing }
            }
            succeed {
                transition: { from: Processing, to: Succeeded }
            }
            fail {
                transition: { from: Processing, to: Failed }
            }
            cancel {
                transition: { from: Pending, to: Cancelled }
                transition: { from: Reserved, to: Cancelled }
                transition: { from: Processing, to: Cancelled }
            }
            deadletter {
                transition: { from: Failed, to: DeadLetter }
            }
            release {
                transition: { from: Reserved, to: Pending }
            }
        }
    }

    pub(super) fn pending() -> TaskLifecycleMachine<(), Pending> {
        TaskLifecycleMachine::new(())
    }

    pub(super) fn reserved() -> TaskLifecycleMachine<(), Reserved> {
        pending()
            .reserve()
            .expect("reserve transition from Pending should exist")
    }

    pub(super) fn processing() -> TaskLifecycleMachine<(), Processing> {
        reserved()
            .start_processing()
            .expect("start_processing transition from Reserved should exist")
    }

    pub(super) fn failed() -> TaskLifecycleMachine<(), Failed> {
        processing()
            .fail()
            .expect("fail transition from Processing should exist")
    }
}

fn invalid_transition(state: &TaskState, event: TaskTransition) -> AppError {
    AppError::Validation(format!(
        "Invalid task transition: {} -> {}",
        state.as_str(),
        event.as_str()
    ))
}

fn compute_next_state(state: &TaskState, event: TaskTransition) -> Result<TaskState, AppError> {
    use lifecycle::*;
    match (state, event) {
        (TaskState::Pending, TaskTransition::Reserve) => pending()
            .reserve()
            .map(|_| TaskState::Reserved)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Failed, TaskTransition::Reserve) => failed()
            .reserve()
            .map(|_| TaskState::Reserved)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Reserved, TaskTransition::StartProcessing) => reserved()
            .start_processing()
            .map(|_| TaskState::Processing)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Processing, TaskTransition::Succeed) => processing()
            .succeed()
            .map(|_| TaskState::Succeeded)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Processing, TaskTransition::Fail) => processing()
            .fail()
            .map(|_| TaskState::Failed)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Pending, TaskTransition::Cancel) => pending()
            .cancel()
            .map(|_| TaskState::Cancelled)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Reserved, TaskTransition::Cancel) => reserved()
            .cancel()
            .map(|_| TaskState::Cancelled)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Processing, TaskTransition::Cancel) => processing()
            .cancel()
            .map(|_| TaskState::Cancelled)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Failed, TaskTransition::DeadLetter) => failed()
            .deadletter()
            .map(|_| TaskState::DeadLetter)
            .map_err(|_| invalid_transition(state, event)),
        (TaskState::Reserved, TaskTransition::Release) => reserved()
            .release()
            .map(|_| TaskState::Pending)
            .map_err(|_| invalid_transition(state, event)),
        _ => Err(invalid_transition(state, event)),
    }
}

stored_object!(IngestionTask, "ingestion_task", {
    content: IngestionPayload,
    state: TaskState,
    user_id: String,
    attempts: u32,
    max_attempts: u32,
    #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime")]
    scheduled_at: chrono::DateTime<chrono::Utc>,
    #[serde(
        serialize_with = "serialize_option_datetime",
        deserialize_with = "deserialize_option_datetime",
        default
    )]
    locked_at: Option<chrono::DateTime<chrono::Utc>>,
    lease_duration_secs: i64,
    worker_id: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    #[serde(
        serialize_with = "serialize_option_datetime",
        deserialize_with = "deserialize_option_datetime",
        default
    )]
    last_error_at: Option<chrono::DateTime<chrono::Utc>>,
    priority: i32
});

impl IngestionTask {
    pub async fn new(content: IngestionPayload, user_id: String) -> Self {
        let now = chrono::Utc::now();

        Self {
            id: Uuid::new_v4().to_string(),
            content,
            state: TaskState::Pending,
            user_id,
            attempts: 0,
            max_attempts: MAX_ATTEMPTS,
            scheduled_at: now,
            locked_at: None,
            lease_duration_secs: DEFAULT_LEASE_SECS,
            worker_id: None,
            error_code: None,
            error_message: None,
            last_error_at: None,
            priority: DEFAULT_PRIORITY,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn can_retry(&self) -> bool {
        self.attempts < self.max_attempts
    }

    pub fn lease_duration(&self) -> Duration {
        Duration::from_secs(self.lease_duration_secs.max(0) as u64)
    }

    pub async fn create_and_add_to_db(
        content: IngestionPayload,
        user_id: String,
        db: &SurrealDbClient,
    ) -> Result<IngestionTask, AppError> {
        let task = Self::new(content, user_id).await;
        db.store_item(task.clone()).await?;
        Ok(task)
    }

    pub async fn claim_next_ready(
        db: &SurrealDbClient,
        worker_id: &str,
        now: chrono::DateTime<chrono::Utc>,
        lease_duration: Duration,
    ) -> Result<Option<IngestionTask>, AppError> {
        debug_assert!(compute_next_state(&TaskState::Pending, TaskTransition::Reserve).is_ok());
        debug_assert!(compute_next_state(&TaskState::Failed, TaskTransition::Reserve).is_ok());

        const CLAIM_QUERY: &str = r#"
            UPDATE (
                SELECT * FROM type::table($table)
                WHERE state IN $candidate_states
                  AND scheduled_at <= $now
                  AND (
                        attempts < max_attempts
                        OR state IN $sticky_states
                  )
                  AND (
                        locked_at = NONE
                        OR time::unix($now) - time::unix(locked_at) >= lease_duration_secs
                  )
                ORDER BY priority DESC, scheduled_at ASC, created_at ASC
                LIMIT 1
            )
            SET state = $reserved_state,
                attempts = if state IN $increment_states THEN
                    if attempts + 1 > max_attempts THEN max_attempts ELSE attempts + 1 END
                ELSE
                    attempts
                END,
                locked_at = $now,
                worker_id = $worker_id,
                lease_duration_secs = $lease_secs,
                updated_at = $now
            RETURN *;
        "#;

        let mut result = db
            .client
            .query(CLAIM_QUERY)
            .bind(("table", Self::table_name()))
            .bind((
                "candidate_states",
                vec![
                    TaskState::Pending.as_str(),
                    TaskState::Failed.as_str(),
                    TaskState::Reserved.as_str(),
                    TaskState::Processing.as_str(),
                ],
            ))
            .bind((
                "sticky_states",
                vec![TaskState::Reserved.as_str(), TaskState::Processing.as_str()],
            ))
            .bind((
                "increment_states",
                vec![TaskState::Pending.as_str(), TaskState::Failed.as_str()],
            ))
            .bind(("reserved_state", TaskState::Reserved.as_str()))
            .bind(("now", SurrealDatetime::from(now)))
            .bind(("worker_id", worker_id.to_string()))
            .bind(("lease_secs", lease_duration.as_secs() as i64))
            .await?;

        let task: Option<IngestionTask> = result.take(0)?;
        Ok(task)
    }

    pub async fn mark_processing(&self, db: &SurrealDbClient) -> Result<IngestionTask, AppError> {
        let next = compute_next_state(&self.state, TaskTransition::StartProcessing)?;
        debug_assert_eq!(next, TaskState::Processing);

        const START_PROCESSING_QUERY: &str = r#"
            UPDATE type::thing($table, $id)
            SET state = $processing,
                updated_at = $now,
                locked_at = $now
            WHERE state = $reserved AND worker_id = $worker_id
            RETURN *;
        "#;

        let now = chrono::Utc::now();
        let mut result = db
            .client
            .query(START_PROCESSING_QUERY)
            .bind(("table", Self::table_name()))
            .bind(("id", self.id.clone()))
            .bind(("processing", TaskState::Processing.as_str()))
            .bind(("reserved", TaskState::Reserved.as_str()))
            .bind(("now", SurrealDatetime::from(now)))
            .bind(("worker_id", self.worker_id.clone().unwrap_or_default()))
            .await?;

        let updated: Option<IngestionTask> = result.take(0)?;
        updated.ok_or_else(|| invalid_transition(&self.state, TaskTransition::StartProcessing))
    }

    pub async fn mark_succeeded(&self, db: &SurrealDbClient) -> Result<IngestionTask, AppError> {
        let next = compute_next_state(&self.state, TaskTransition::Succeed)?;
        debug_assert_eq!(next, TaskState::Succeeded);

        const COMPLETE_QUERY: &str = r#"
            UPDATE type::thing($table, $id)
            SET state = $succeeded,
                updated_at = $now,
                locked_at = NONE,
                worker_id = NONE,
                scheduled_at = $now,
                error_code = NONE,
                error_message = NONE,
                last_error_at = NONE
            WHERE state = $processing AND worker_id = $worker_id
            RETURN *;
        "#;

        let now = chrono::Utc::now();
        let mut result = db
            .client
            .query(COMPLETE_QUERY)
            .bind(("table", Self::table_name()))
            .bind(("id", self.id.clone()))
            .bind(("succeeded", TaskState::Succeeded.as_str()))
            .bind(("processing", TaskState::Processing.as_str()))
            .bind(("now", SurrealDatetime::from(now)))
            .bind(("worker_id", self.worker_id.clone().unwrap_or_default()))
            .await?;

        let updated: Option<IngestionTask> = result.take(0)?;
        updated.ok_or_else(|| invalid_transition(&self.state, TaskTransition::Succeed))
    }

    pub async fn mark_failed(
        &self,
        error: TaskErrorInfo,
        retry_delay: Duration,
        db: &SurrealDbClient,
    ) -> Result<IngestionTask, AppError> {
        let next = compute_next_state(&self.state, TaskTransition::Fail)?;
        debug_assert_eq!(next, TaskState::Failed);

        let now = chrono::Utc::now();
        let retry_at = now
            + ChronoDuration::from_std(retry_delay).unwrap_or_else(|_| ChronoDuration::seconds(30));

        const FAIL_QUERY: &str = r#"
            UPDATE type::thing($table, $id)
            SET state = $failed,
                updated_at = $now,
                locked_at = NONE,
                worker_id = NONE,
                scheduled_at = $retry_at,
                error_code = $error_code,
                error_message = $error_message,
                last_error_at = $now
            WHERE state = $processing AND worker_id = $worker_id
            RETURN *;
        "#;

        let mut result = db
            .client
            .query(FAIL_QUERY)
            .bind(("table", Self::table_name()))
            .bind(("id", self.id.clone()))
            .bind(("failed", TaskState::Failed.as_str()))
            .bind(("processing", TaskState::Processing.as_str()))
            .bind(("now", SurrealDatetime::from(now)))
            .bind(("retry_at", SurrealDatetime::from(retry_at)))
            .bind(("error_code", error.code.clone()))
            .bind(("error_message", error.message.clone()))
            .bind(("worker_id", self.worker_id.clone().unwrap_or_default()))
            .await?;

        let updated: Option<IngestionTask> = result.take(0)?;
        updated.ok_or_else(|| invalid_transition(&self.state, TaskTransition::Fail))
    }

    pub async fn mark_dead_letter(
        &self,
        error: TaskErrorInfo,
        db: &SurrealDbClient,
    ) -> Result<IngestionTask, AppError> {
        let next = compute_next_state(&self.state, TaskTransition::DeadLetter)?;
        debug_assert_eq!(next, TaskState::DeadLetter);

        const DEAD_LETTER_QUERY: &str = r#"
            UPDATE type::thing($table, $id)
            SET state = $dead,
                updated_at = $now,
                locked_at = NONE,
                worker_id = NONE,
                scheduled_at = $now,
                error_code = $error_code,
                error_message = $error_message,
                last_error_at = $now
            WHERE state = $failed
            RETURN *;
        "#;

        let now = chrono::Utc::now();
        let mut result = db
            .client
            .query(DEAD_LETTER_QUERY)
            .bind(("table", Self::table_name()))
            .bind(("id", self.id.clone()))
            .bind(("dead", TaskState::DeadLetter.as_str()))
            .bind(("failed", TaskState::Failed.as_str()))
            .bind(("now", SurrealDatetime::from(now)))
            .bind(("error_code", error.code.clone()))
            .bind(("error_message", error.message.clone()))
            .await?;

        let updated: Option<IngestionTask> = result.take(0)?;
        updated.ok_or_else(|| invalid_transition(&self.state, TaskTransition::DeadLetter))
    }

    pub async fn mark_cancelled(&self, db: &SurrealDbClient) -> Result<IngestionTask, AppError> {
        compute_next_state(&self.state, TaskTransition::Cancel)?;

        const CANCEL_QUERY: &str = r#"
            UPDATE type::thing($table, $id)
            SET state = $cancelled,
                updated_at = $now,
                locked_at = NONE,
                worker_id = NONE
            WHERE state IN $allow_states
            RETURN *;
        "#;

        let now = chrono::Utc::now();
        let mut result = db
            .client
            .query(CANCEL_QUERY)
            .bind(("table", Self::table_name()))
            .bind(("id", self.id.clone()))
            .bind(("cancelled", TaskState::Cancelled.as_str()))
            .bind((
                "allow_states",
                vec![
                    TaskState::Pending.as_str(),
                    TaskState::Reserved.as_str(),
                    TaskState::Processing.as_str(),
                ],
            ))
            .bind(("now", SurrealDatetime::from(now)))
            .await?;

        let updated: Option<IngestionTask> = result.take(0)?;
        updated.ok_or_else(|| invalid_transition(&self.state, TaskTransition::Cancel))
    }

    pub async fn release(&self, db: &SurrealDbClient) -> Result<IngestionTask, AppError> {
        compute_next_state(&self.state, TaskTransition::Release)?;

        const RELEASE_QUERY: &str = r#"
            UPDATE type::thing($table, $id)
            SET state = $pending,
                updated_at = $now,
                locked_at = NONE,
                worker_id = NONE
            WHERE state = $reserved
            RETURN *;
        "#;

        let now = chrono::Utc::now();
        let mut result = db
            .client
            .query(RELEASE_QUERY)
            .bind(("table", Self::table_name()))
            .bind(("id", self.id.clone()))
            .bind(("pending", TaskState::Pending.as_str()))
            .bind(("reserved", TaskState::Reserved.as_str()))
            .bind(("now", SurrealDatetime::from(now)))
            .await?;

        let updated: Option<IngestionTask> = result.take(0)?;
        updated.ok_or_else(|| invalid_transition(&self.state, TaskTransition::Release))
    }

    pub async fn get_unfinished_tasks(
        db: &SurrealDbClient,
    ) -> Result<Vec<IngestionTask>, AppError> {
        let tasks: Vec<IngestionTask> = db
            .query(
                "SELECT * FROM type::table($table)
                 WHERE state IN $active_states
                 ORDER BY scheduled_at ASC, created_at ASC",
            )
            .bind(("table", Self::table_name()))
            .bind((
                "active_states",
                vec![
                    TaskState::Pending.as_str(),
                    TaskState::Reserved.as_str(),
                    TaskState::Processing.as_str(),
                    TaskState::Failed.as_str(),
                ],
            ))
            .await?
            .take(0)?;

        Ok(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::ingestion_payload::IngestionPayload;

    fn create_payload(user_id: &str) -> IngestionPayload {
        IngestionPayload::Text {
            text: "Test content".to_string(),
            context: "Test context".to_string(),
            category: "Test category".to_string(),
            user_id: user_id.to_string(),
        }
    }

    async fn memory_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = Uuid::new_v4().to_string();
        SurrealDbClient::memory(namespace, &database)
            .await
            .expect("in-memory surrealdb")
    }

    #[tokio::test]
    async fn test_new_task_defaults() {
        let user_id = "user123";
        let payload = create_payload(user_id);
        let task = IngestionTask::new(payload.clone(), user_id.to_string()).await;

        assert_eq!(task.user_id, user_id);
        assert_eq!(task.content, payload);
        assert_eq!(task.state, TaskState::Pending);
        assert_eq!(task.attempts, 0);
        assert_eq!(task.max_attempts, MAX_ATTEMPTS);
        assert!(task.locked_at.is_none());
        assert!(task.worker_id.is_none());
    }

    #[tokio::test]
    async fn test_create_and_store_task() {
        let db = memory_db().await;
        let user_id = "user123";
        let payload = create_payload(user_id);

        let created =
            IngestionTask::create_and_add_to_db(payload.clone(), user_id.to_string(), &db)
                .await
                .expect("store");

        let stored: Option<IngestionTask> = db
            .get_item::<IngestionTask>(&created.id)
            .await
            .expect("fetch");

        let stored = stored.expect("task exists");
        assert_eq!(stored.id, created.id);
        assert_eq!(stored.state, TaskState::Pending);
        assert_eq!(stored.attempts, 0);
    }

    #[tokio::test]
    async fn test_claim_and_transition() {
        let db = memory_db().await;
        let user_id = "user123";
        let payload = create_payload(user_id);
        let task = IngestionTask::new(payload, user_id.to_string()).await;
        db.store_item(task.clone()).await.expect("store");

        let worker_id = "worker-1";
        let now = chrono::Utc::now();
        let claimed = IngestionTask::claim_next_ready(&db, worker_id, now, Duration::from_secs(60))
            .await
            .expect("claim");

        let claimed = claimed.expect("task claimed");
        assert_eq!(claimed.state, TaskState::Reserved);
        assert_eq!(claimed.worker_id.as_deref(), Some(worker_id));

        let processing = claimed.mark_processing(&db).await.expect("processing");
        assert_eq!(processing.state, TaskState::Processing);

        let succeeded = processing.mark_succeeded(&db).await.expect("succeeded");
        assert_eq!(succeeded.state, TaskState::Succeeded);
        assert!(succeeded.worker_id.is_none());
        assert!(succeeded.locked_at.is_none());
    }

    #[tokio::test]
    async fn test_fail_and_dead_letter() {
        let db = memory_db().await;
        let user_id = "user123";
        let payload = create_payload(user_id);
        let task = IngestionTask::new(payload, user_id.to_string()).await;
        db.store_item(task.clone()).await.expect("store");

        let worker_id = "worker-dead";
        let now = chrono::Utc::now();
        let claimed = IngestionTask::claim_next_ready(&db, worker_id, now, Duration::from_secs(60))
            .await
            .expect("claim")
            .expect("claimed");

        let processing = claimed.mark_processing(&db).await.expect("processing");

        let error_info = TaskErrorInfo {
            code: Some("pipeline_error".into()),
            message: "failed".into(),
        };

        let failed = processing
            .mark_failed(error_info.clone(), Duration::from_secs(30), &db)
            .await
            .expect("failed update");
        assert_eq!(failed.state, TaskState::Failed);
        assert_eq!(failed.error_message.as_deref(), Some("failed"));
        assert!(failed.worker_id.is_none());
        assert!(failed.locked_at.is_none());
        assert!(failed.scheduled_at > now);

        let dead = failed
            .mark_dead_letter(error_info.clone(), &db)
            .await
            .expect("dead letter");
        assert_eq!(dead.state, TaskState::DeadLetter);
        assert_eq!(dead.error_message.as_deref(), Some("failed"));
    }
}
