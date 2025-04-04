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
}

impl fmt::Display for MessageRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageRole::User => write!(f, "User"),
            MessageRole::AI => write!(f, "AI"),
            MessageRole::System => write!(f, "System"),
        }
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.role, self.content)
    }
}

// helper function to format a vector of messages
pub fn format_history(history: &[Message]) -> String {
    history
        .iter()
        .map(|msg| format!("{}", msg))
        .collect::<Vec<String>>()
        .join("\n")
}
