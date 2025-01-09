use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::{ingress::types::ingress_object::IngressObject, stored_object};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    Created,
    InProgress {
        attempts: u32,
        last_attempt: String, // timestamp
    },
    Completed,
    Error(String),
    Cancelled,
}

stored_object!(Job, "job", {
    content: IngressObject,
    status: JobStatus,
    created_at: String,
    updated_at: String,
    user_id: String
});

impl Job {
    pub async fn new(content: IngressObject, user_id: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string();

        Self {
            id: Uuid::new_v4().to_string(),
            content,
            status: JobStatus::Created,
            created_at: now.clone(),
            updated_at: now,
            user_id,
        }
    }
}
