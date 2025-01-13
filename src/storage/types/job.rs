use uuid::Uuid;

use crate::{ingress::types::ingress_object::IngressObject, stored_object};

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
}
