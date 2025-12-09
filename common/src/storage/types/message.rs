#![allow(clippy::module_name_repetitions)]
use uuid::Uuid;

use crate::stored_object;

#[derive(Deserialize, Debug, Clone, Serialize, PartialEq)]
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
        .map(|msg| format!("{msg}"))
        .collect::<Vec<String>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::SurrealDbClient;

    #[tokio::test]
    async fn test_message_creation() {
        // Test basic message creation
        let conversation_id = "test_conversation";
        let content = "This is a test message";
        let role = MessageRole::User;
        let references = Some(vec!["ref1".to_string(), "ref2".to_string()]);

        let message = Message::new(
            conversation_id.to_string(),
            role.clone(),
            content.to_string(),
            references.clone(),
        );

        // Verify message properties
        assert_eq!(message.conversation_id, conversation_id);
        assert_eq!(message.content, content);
        assert_eq!(message.role, role);
        assert_eq!(message.references, references);
        assert!(!message.id.is_empty());
    }

    #[tokio::test]
    async fn test_message_persistence() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &uuid::Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create and store a message
        let conversation_id = "test_conversation";
        let message = Message::new(
            conversation_id.to_string(),
            MessageRole::User,
            "Hello world".to_string(),
            None,
        );
        let message_id = message.id.clone();

        // Store the message
        db.store_item(message.clone())
            .await
            .expect("Failed to store message");

        // Retrieve the message
        let retrieved: Option<Message> = db
            .get_item(&message_id)
            .await
            .expect("Failed to retrieve message");

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();

        // Verify retrieved properties match original
        assert_eq!(retrieved.id, message.id);
        assert_eq!(retrieved.conversation_id, message.conversation_id);
        assert_eq!(retrieved.role, message.role);
        assert_eq!(retrieved.content, message.content);
        assert_eq!(retrieved.references, message.references);
    }

    #[tokio::test]
    async fn test_message_role_display() {
        // Test the Display implementation for MessageRole
        assert_eq!(format!("{}", MessageRole::User), "User");
        assert_eq!(format!("{}", MessageRole::AI), "AI");
        assert_eq!(format!("{}", MessageRole::System), "System");
    }

    #[tokio::test]
    async fn test_message_display() {
        // Test the Display implementation for Message
        let message = Message {
            id: "test_id".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            conversation_id: "test_convo".to_string(),
            role: MessageRole::User,
            content: "Hello world".to_string(),
            references: None,
        };

        assert_eq!(format!("{}", message), "User: Hello world");
    }

    #[tokio::test]
    async fn test_format_history() {
        // Create a vector of messages
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

        // Format the history
        let formatted = format_history(&messages);

        // Verify the formatting
        assert_eq!(formatted, "User: Hello\nAI: Hi there!");
    }
}
