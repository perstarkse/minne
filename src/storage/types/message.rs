use uuid::Uuid;

use crate::stored_object;

#[derive(Deserialize, Debug, Clone, Serialize)]
pub enum MessageRole {
    User,
    AI,
    System,
}

stored_object!(Message, "message", {
    conversation_id: String,
    role: MessageRole,
    content: String,
    references: Option<Vec<String>>
});

impl Message {
    pub fn new(
        conversation_id: String,
        role: MessageRole,
        content: String,
        references: Option<Vec<String>>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            conversation_id,
            role,
            content,
            references,
        }
    }
    pub fn new_ai_message(
        conversation_id: String,
        id: String,
        content: String,
        references: Option<Vec<String>>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            created_at: now,
            updated_at: now,
            role: MessageRole::AI,
            content,
            references,
            conversation_id,
        }
    }
}
