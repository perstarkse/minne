#![allow(clippy::module_name_repetitions)]
use uuid::Uuid;

use std::fmt;
use std::fmt::Write;

use crate::stored_object;

#[derive(Deserialize, Debug, Clone, Copy, Serialize, PartialEq)]
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
    let mut out = String::new();
    for (i, msg) in history.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write!(out, "{msg}").unwrap_or_default();
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use super::*;
    use crate::storage::db::SurrealDbClient;
    use anyhow::{self, Context};

    #[tokio::test]
    async fn test_message_creation() -> anyhow::Result<()> {
        let conversation_id = "test_conversation";
        let content = "This is a test message";
        let role = MessageRole::User;
        let references = Some(vec!["ref1".to_string(), "ref2".to_string()]);

        let message = Message::new(
            conversation_id.to_string(),
            role,
            content.to_string(),
            references.clone(),
        );

        assert_eq!(message.conversation_id, conversation_id);
        assert_eq!(message.content, content);
        assert_eq!(message.role, role);
        assert_eq!(message.references, references);
        assert!(!message.id.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_message_persistence() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &uuid::Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        let conversation_id = "test_conversation";
        let message = Message::new(
            conversation_id.to_string(),
            MessageRole::User,
            "Hello world".to_string(),
            None,
        );
        let message_id = message.id.clone();

        db.store_item(message.clone())
            .await
            .with_context(|| "Failed to store message".to_string())?;

        let retrieved: Option<Message> = db
            .get_item(&message_id)
            .await
            .with_context(|| "Failed to retrieve message".to_string())?;

        let retrieved = retrieved.ok_or_else(|| anyhow::anyhow!("Expected message to exist"))?;

        assert_eq!(retrieved.id, message.id);
        assert_eq!(retrieved.conversation_id, message.conversation_id);
        assert_eq!(retrieved.role, message.role);
        assert_eq!(retrieved.content, message.content);
        assert_eq!(retrieved.references, message.references);

        Ok(())
    }

    #[tokio::test]
    async fn test_message_role_display() -> anyhow::Result<()> {
        assert_eq!(format!("{}", MessageRole::User), "User");
        assert_eq!(format!("{}", MessageRole::AI), "AI");
        assert_eq!(format!("{}", MessageRole::System), "System");

        Ok(())
    }

    #[tokio::test]
    async fn test_message_display() -> anyhow::Result<()> {
        let message = Message {
            id: "test_id".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            conversation_id: "test_convo".to_string(),
            role: MessageRole::User,
            content: "Hello world".to_string(),
            references: None,
        };

        assert_eq!(format!("{message}"), "User: Hello world");

        Ok(())
    }

    #[tokio::test]
    async fn test_format_history() -> anyhow::Result<()> {
        let messages = vec![
            Message {
                id: "1".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                conversation_id: "test_convo".to_string(),
                role: MessageRole::User,
                content: "Hello".to_string(),
                references: None,
            },
            Message {
                id: "2".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                conversation_id: "test_convo".to_string(),
                role: MessageRole::AI,
                content: "Hi there!".to_string(),
                references: None,
            },
        ];

        let formatted = format_history(&messages);

        assert_eq!(formatted, "User: Hello\nAI: Hi there!");

        Ok(())
    }
}
