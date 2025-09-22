use surrealdb::opt::PatchOp;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::message::Message;

stored_object!(Conversation, "conversation", {
    user_id: String,
    title: String
});

impl Conversation {
    pub fn new(user_id: String, title: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            user_id,
            title,
        }
    }

    pub async fn get_complete_conversation(
        conversation_id: &str,
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<(Self, Vec<Message>), AppError> {
        let conversation: Conversation = db
            .get_item(conversation_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Conversation not found".to_string()))?;

        if conversation.user_id != user_id {
            return Err(AppError::Auth(
                "You don't have access to this conversation".to_string(),
            ));
        }

        let messages:Vec<Message> = db.client.
            query("SELECT * FROM type::table($table_name) WHERE conversation_id = $conversation_id ORDER BY updated_at").
            bind(("table_name", Message::table_name())).
            bind(("conversation_id", conversation_id.to_string()))
            .await?
            .take(0)?;

        Ok((conversation, messages))
    }
    pub async fn patch_title(
        id: &str,
        user_id: &str,
        new_title: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        // First verify ownership by getting conversation user_id
        let conversation: Option<Conversation> = db.get_item(id).await?;
        let conversation =
            conversation.ok_or_else(|| AppError::NotFound("Conversation not found".to_string()))?;

        if conversation.user_id != user_id {
            return Err(AppError::Auth(
                "Unauthorized to update this conversation".to_string(),
            ));
        }

        let _updated: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/title", new_title.to_string()))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::Datetime::from(Utc::now()),
            ))
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::types::message::MessageRole;

    use super::*;

    #[tokio::test]
    async fn test_create_conversation() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a new conversation
        let user_id = "test_user";
        let title = "Test Conversation";
        let conversation = Conversation::new(user_id.to_string(), title.to_string());

        // Verify conversation properties
        assert_eq!(conversation.user_id, user_id);
        assert_eq!(conversation.title, title);
        assert!(!conversation.id.is_empty());

        // Store the conversation
        let result = db.store_item(conversation.clone()).await;
        assert!(result.is_ok());

        // Verify it can be retrieved
        let retrieved: Option<Conversation> = db
            .get_item(&conversation.id)
            .await
            .expect("Failed to retrieve conversation");
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, conversation.id);
        assert_eq!(retrieved.user_id, user_id);
        assert_eq!(retrieved.title, title);
    }

    #[tokio::test]
    async fn test_get_complete_conversation_not_found() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Try to get a conversation that doesn't exist
        let result =
            Conversation::get_complete_conversation("nonexistent_id", "test_user", &db).await;
        assert!(result.is_err());

        match result {
            Err(AppError::NotFound(_)) => { /* expected error */ }
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_get_complete_conversation_unauthorized() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create and store a conversation for user_id_1
        let user_id_1 = "user_1";
        let conversation =
            Conversation::new(user_id_1.to_string(), "Private Conversation".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .expect("Failed to store conversation");

        // Try to access with a different user
        let user_id_2 = "user_2";
        let result =
            Conversation::get_complete_conversation(&conversation_id, user_id_2, &db).await;
        assert!(result.is_err());

        match result {
            Err(AppError::Auth(_)) => { /* expected error */ }
            _ => panic!("Expected Auth error"),
        }
    }

    #[tokio::test]
    async fn test_patch_title_success() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let user_id = "user_1";
        let original_title = "Original Title";
        let conversation = Conversation::new(user_id.to_string(), original_title.to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .expect("Failed to store conversation");

        let new_title = "Updated Title";

        // Patch title successfully
        let result = Conversation::patch_title(&conversation_id, user_id, new_title, &db).await;
        assert!(result.is_ok());

        // Retrieve from DB to verify
        let updated_conversation = db
            .get_item::<Conversation>(&conversation_id)
            .await
            .expect("Failed to get conversation")
            .expect("Conversation missing");
        assert_eq!(updated_conversation.title, new_title);
        assert_eq!(updated_conversation.user_id, user_id);
    }

    #[tokio::test]
    async fn test_patch_title_not_found() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Try to patch non-existing conversation
        let result = Conversation::patch_title("nonexistent", "user_x", "New Title", &db).await;

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_patch_title_unauthorized() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let owner_id = "owner";
        let other_user_id = "intruder";
        let conversation = Conversation::new(owner_id.to_string(), "Private".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .expect("Failed to store conversation");

        // Attempt patch with unauthorized user
        let result =
            Conversation::patch_title(&conversation_id, other_user_id, "Hacked Title", &db).await;

        assert!(result.is_err());
        match result {
            Err(AppError::Auth(_)) => {}
            _ => panic!("Expected Auth error"),
        }
    }

    #[tokio::test]
    async fn test_get_complete_conversation_with_messages() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create and store a conversation for user_id_1
        let user_id_1 = "user_1";
        let conversation = Conversation::new(user_id_1.to_string(), "Conversation".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .expect("Failed to store conversation");

        // Create messages
        let message1 = Message::new(
            conversation_id.clone(),
            MessageRole::User,
            "Hello, AI!".to_string(),
            None,
        );
        let message2 = Message::new(
            conversation_id.clone(),
            MessageRole::AI,
            "Hello, human! How can I help you today?".to_string(),
            None,
        );
        let message3 = Message::new(
            conversation_id.clone(),
            MessageRole::User,
            "Tell me about Rust programming.".to_string(),
            None,
        );

        // Store messages
        db.store_item(message1)
            .await
            .expect("Failed to store message1");
        db.store_item(message2)
            .await
            .expect("Failed to store message2");
        db.store_item(message3)
            .await
            .expect("Failed to store message3");

        // Retrieve the complete conversation
        let result =
            Conversation::get_complete_conversation(&conversation_id, user_id_1, &db).await;
        assert!(result.is_ok(), "Failed to retrieve complete conversation");

        let (retrieved_conversation, messages) = result.unwrap();

        // Verify conversation data
        assert_eq!(retrieved_conversation.id, conversation_id);
        assert_eq!(retrieved_conversation.user_id, user_id_1);
        assert_eq!(retrieved_conversation.title, "Conversation");

        // Verify messages
        assert_eq!(messages.len(), 3);

        // Verify messages are sorted by updated_at
        let message_contents: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
        assert!(message_contents.contains(&"Hello, AI!"));
        assert!(message_contents.contains(&"Hello, human! How can I help you today?"));
        assert!(message_contents.contains(&"Tell me about Rust programming."));

        // Make sure we can't access with different user
        let user_id_2 = "user_2";
        let unauthorized_result =
            Conversation::get_complete_conversation(&conversation_id, user_id_2, &db).await;
        assert!(unauthorized_result.is_err());
        match unauthorized_result {
            Err(AppError::Auth(_)) => { /* expected error */ }
            _ => panic!("Expected Auth error"),
        }
    }
}
