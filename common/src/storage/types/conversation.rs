use surrealdb::opt::PatchOp;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::message::Message;

stored_object!(Conversation, "conversation", {
    user_id: String,
    title: String
});

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub struct SidebarConversation {
    #[serde(deserialize_with = "deserialize_sidebar_id")]
    pub id: String,
    pub title: String,
}

struct SidebarIdVisitor;

impl<'de> serde::de::Visitor<'de> for SidebarIdVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string id or a SurrealDB Thing")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(value.to_string())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(value)
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let thing = <surrealdb::sql::Thing as serde::Deserialize>::deserialize(
            serde::de::value::MapAccessDeserializer::new(map),
        )?;
        Ok(thing.id.to_raw())
    }
}

fn deserialize_sidebar_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(SidebarIdVisitor)
}

impl Conversation {
    #[must_use]
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
            .ok_or_else(|| AppError::NotFound("conversation not found".to_string()))?;

        if conversation.user_id != user_id {
            return Err(AppError::Auth(
                "You don't have access to this conversation".to_string(),
            ));
        }

        let messages: Vec<Message> = db
            .client
            .query(
                "SELECT * FROM type::table($message_table) WHERE conversation_id = $conversation_id AND type::thing($conversation_table, $conversation_id).user_id = $user_id ORDER BY updated_at",
            )
            .bind(("message_table", Message::table_name()))
            .bind(("conversation_table", Self::table_name()))
            .bind(("conversation_id", conversation_id.to_string()))
            .bind(("user_id", user_id.to_string()))
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
            conversation.ok_or_else(|| AppError::NotFound("conversation not found".to_string()))?;

        if conversation.user_id != user_id {
            return Err(AppError::Auth(
                "Unauthorized to update this conversation".to_string(),
            ));
        }

        let updated: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/title", new_title.to_string()))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::Datetime::from(Utc::now()),
            ))
            .await?;

        if updated.is_none() {
            return Err(AppError::NotFound("conversation not found".to_string()));
        }

        Ok(())
    }

    pub async fn get_user_sidebar_conversations(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<SidebarConversation>, AppError> {
        let conversations: Vec<SidebarConversation> = db
            .client
            .query(
                "SELECT id, title, updated_at FROM type::table($table_name) WHERE user_id = $user_id ORDER BY updated_at DESC",
            )
            .bind(("table_name", Self::table_name()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;

        Ok(conversations)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use crate::storage::types::message::MessageRole;
    use crate::test_utils::setup_test_db;
    use anyhow::{self, Context};

    use super::*;

    const MESSAGE_QUERY_FOR_OWNER: &str = "SELECT * FROM type::table($message_table) WHERE conversation_id = $conversation_id AND type::thing($conversation_table, $conversation_id).user_id = $user_id ORDER BY updated_at";

    async fn fetch_messages_for_owner(
        db: &SurrealDbClient,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<Vec<Message>, AppError> {
        db.client
            .query(MESSAGE_QUERY_FOR_OWNER)
            .bind(("message_table", Message::table_name()))
            .bind(("conversation_table", Conversation::table_name()))
            .bind(("conversation_id", conversation_id.to_string()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)
            .map_err(AppError::from)
    }

    #[tokio::test]
    async fn test_create_conversation() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let user_id = "test_user";
        let title = "Test Conversation";
        let conversation = Conversation::new(user_id.to_string(), title.to_string());

        assert_eq!(conversation.user_id, user_id);
        assert_eq!(conversation.title, title);
        assert!(!conversation.id.is_empty());

        let result = db.store_item(conversation.clone()).await;
        assert!(result.is_ok());

        let retrieved: Option<Conversation> = db
            .get_item(&conversation.id)
            .await
            .with_context(|| "Failed to retrieve conversation".to_string())?;

        let retrieved =
            retrieved.ok_or_else(|| anyhow::anyhow!("Expected conversation to exist"))?;
        assert_eq!(retrieved.id, conversation.id);
        assert_eq!(retrieved.user_id, user_id);
        assert_eq!(retrieved.title, title);

        Ok(())
    }

    #[tokio::test]
    async fn test_get_complete_conversation_not_found() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let result =
            Conversation::get_complete_conversation("nonexistent_id", "test_user", &db).await;
        assert!(result.is_err());

        match result {
            Err(AppError::NotFound(_)) => {}
            _ => anyhow::bail!("Expected NotFound error"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_get_complete_conversation_unauthorized() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let user_id_1 = "user_1";
        let conversation =
            Conversation::new(user_id_1.to_string(), "Private Conversation".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .with_context(|| "Failed to store conversation".to_string())?;

        let user_id_2 = "user_2";
        let result =
            Conversation::get_complete_conversation(&conversation_id, user_id_2, &db).await;
        assert!(result.is_err());

        match result {
            Err(AppError::Auth(_)) => {}
            _ => anyhow::bail!("Expected Auth error"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_patch_title_success() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let user_id = "user_1";
        let original_title = "Original Title";
        let conversation = Conversation::new(user_id.to_string(), original_title.to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .with_context(|| "Failed to store conversation".to_string())?;

        let new_title = "Updated Title";

        let result = Conversation::patch_title(&conversation_id, user_id, new_title, &db).await;
        assert!(result.is_ok());

        let updated_conversation = db
            .get_item::<Conversation>(&conversation_id)
            .await
            .with_context(|| "Failed to get conversation".to_string())?
            .ok_or_else(|| anyhow::anyhow!("Conversation missing"))?;
        assert_eq!(updated_conversation.title, new_title);
        assert_eq!(updated_conversation.user_id, user_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_patch_title_not_found() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let result = Conversation::patch_title("nonexistent", "user_x", "New Title", &db).await;

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(_)) => {}
            _ => anyhow::bail!("Expected NotFound error"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_patch_title_unauthorized() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let owner_id = "owner";
        let other_user_id = "intruder";
        let conversation = Conversation::new(owner_id.to_string(), "Private".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .with_context(|| "Failed to store conversation".to_string())?;

        let result =
            Conversation::patch_title(&conversation_id, other_user_id, "Hacked Title", &db).await;

        assert!(result.is_err());
        match result {
            Err(AppError::Auth(_)) => {}
            _ => anyhow::bail!("Expected Auth error"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_get_user_sidebar_conversations_filters_and_orders_by_updated_at_desc() {
        let db = setup_test_db().await.expect("setup_test_db");

        let user_id = "sidebar_user";
        let other_user_id = "other_user";
        let base = Utc::now();

        let mut oldest = Conversation::new(user_id.to_string(), "Oldest".to_string());
        oldest.updated_at = base - chrono::Duration::minutes(30);

        let mut newest = Conversation::new(user_id.to_string(), "Newest".to_string());
        newest.updated_at = base - chrono::Duration::minutes(5);

        let mut middle = Conversation::new(user_id.to_string(), "Middle".to_string());
        middle.updated_at = base - chrono::Duration::minutes(15);

        let mut other_user = Conversation::new(other_user_id.to_string(), "Other".to_string());
        other_user.updated_at = base;

        db.store_item(oldest.clone())
            .await
            .expect("Failed to store oldest conversation");
        db.store_item(newest.clone())
            .await
            .expect("Failed to store newest conversation");
        db.store_item(middle.clone())
            .await
            .expect("Failed to store middle conversation");
        db.store_item(other_user)
            .await
            .expect("Failed to store other-user conversation");

        let sidebar_items = Conversation::get_user_sidebar_conversations(user_id, &db)
            .await
            .expect("Failed to get sidebar conversations");

        assert_eq!(sidebar_items.len(), 3);
        let s0 = sidebar_items.first().expect("expected 3 items");
        let s1 = sidebar_items.get(1).expect("expected 3 items");
        let s2 = sidebar_items.get(2).expect("expected 3 items");
        assert_eq!(s0.id, newest.id);
        assert_eq!(s0.title, "Newest");
        assert_eq!(s1.id, middle.id);
        assert_eq!(s1.title, "Middle");
        assert_eq!(s2.id, oldest.id);
        assert_eq!(s2.title, "Oldest");
    }

    #[tokio::test]
    async fn test_sidebar_projection_reflects_patch_title_and_updated_at_reorder() {
        let db = setup_test_db().await.expect("setup_test_db");

        let user_id = "sidebar_patch_user";
        let base = Utc::now();

        let mut first = Conversation::new(user_id.to_string(), "First".to_string());
        first.updated_at = base - chrono::Duration::minutes(20);

        let mut second = Conversation::new(user_id.to_string(), "Second".to_string());
        second.updated_at = base - chrono::Duration::minutes(10);

        db.store_item(first.clone())
            .await
            .expect("Failed to store first conversation");
        db.store_item(second.clone())
            .await
            .expect("Failed to store second conversation");

        let before_patch = Conversation::get_user_sidebar_conversations(user_id, &db)
            .await
            .expect("Failed to get sidebar conversations before patch");
        let before = before_patch.first().expect("expected at least 1 item");
        assert_eq!(before.id, second.id);

        Conversation::patch_title(&first.id, user_id, "First (renamed)", &db)
            .await
            .expect("Failed to patch conversation title");

        let after_patch = Conversation::get_user_sidebar_conversations(user_id, &db)
            .await
            .expect("Failed to get sidebar conversations after patch");
        let after = after_patch.first().expect("expected at least 1 item");
        assert_eq!(after.id, first.id);
        assert_eq!(after.title, "First (renamed)");
    }

    #[tokio::test]
    async fn test_get_complete_conversation_with_messages() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let user_id_1 = "user_1";
        let conversation = Conversation::new(user_id_1.to_string(), "Conversation".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation)
            .await
            .with_context(|| "Failed to store conversation".to_string())?;

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

        db.store_item(message1)
            .await
            .with_context(|| "Failed to store message1".to_string())?;
        db.store_item(message2)
            .await
            .with_context(|| "Failed to store message2".to_string())?;
        db.store_item(message3)
            .await
            .with_context(|| "Failed to store message3".to_string())?;

        let result =
            Conversation::get_complete_conversation(&conversation_id, user_id_1, &db).await;
        assert!(result.is_ok(), "Failed to retrieve complete conversation");

        let (retrieved_conversation, retrieved_messages) =
            result.with_context(|| "Failed to retrieve complete conversation".to_string())?;

        assert_eq!(retrieved_conversation.id, conversation_id);
        assert_eq!(retrieved_conversation.user_id, user_id_1);
        assert_eq!(retrieved_conversation.title, "Conversation");

        assert_eq!(retrieved_messages.len(), 3);

        let message_contents: Vec<&str> = retrieved_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect();
        assert!(message_contents.contains(&"Hello, AI!"));
        assert!(message_contents.contains(&"Hello, human! How can I help you today?"));
        assert!(message_contents.contains(&"Tell me about Rust programming."));

        let user_id_2 = "user_2";
        let unauthorized_result =
            Conversation::get_complete_conversation(&conversation_id, user_id_2, &db).await;
        assert!(unauthorized_result.is_err());
        match unauthorized_result {
            Err(AppError::Auth(_)) => {}
            _ => anyhow::bail!("Expected Auth error"),
        }

        Ok(())
    }

    #[test]
    fn test_sidebar_conversation_deserializes_plain_string_id() {
        let item: SidebarConversation =
            serde_json::from_str(r#"{"id":"conv-plain","title":"My chat"}"#)
                .expect("valid sidebar conversation json");
        assert_eq!(item.id, "conv-plain");
        assert_eq!(item.title, "My chat");
    }

    #[tokio::test]
    async fn test_sidebar_conversation_deserializes_id_from_db_record() {
        let db = setup_test_db().await.expect("setup_test_db");

        let owner = "sidebar_owner";
        let conversation = Conversation::new(owner.to_string(), "Sidebar title".to_string());
        let expected_id = conversation.id.clone();
        db.store_item(conversation)
            .await
            .expect("Failed to store conversation");

        let items = Conversation::get_user_sidebar_conversations(owner, &db)
            .await
            .expect("Failed to load sidebar");
        assert_eq!(items.len(), 1);
        let item = items.first().expect("expected one sidebar item");
        assert_eq!(item.id, expected_id);
        assert_eq!(item.title, "Sidebar title");
    }

    #[tokio::test]
    async fn test_message_query_filters_by_owner_user_id_in_sql() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let owner = "owner_user";
        let intruder = "intruder_user";
        let conversation = Conversation::new(owner.to_string(), "Private".to_string());
        let conversation_id = conversation.id.clone();

        db.store_item(conversation).await?;
        db.store_item(Message::new(
            conversation_id.clone(),
            MessageRole::User,
            "secret message".to_string(),
            None,
        ))
        .await?;

        let owner_messages = fetch_messages_for_owner(&db, &conversation_id, owner).await?;
        assert_eq!(owner_messages.len(), 1);
        assert_eq!(
            owner_messages
                .first()
                .expect("expected owner message")
                .content,
            "secret message"
        );

        let intruder_messages = fetch_messages_for_owner(&db, &conversation_id, intruder).await?;
        assert!(
            intruder_messages.is_empty(),
            "SQL owner filter must not return messages for a non-owner user_id"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_get_complete_conversation_orders_messages_by_updated_at() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let user_id = "order_user";
        let conversation = Conversation::new(user_id.to_string(), "Ordered".to_string());
        let conversation_id = conversation.id.clone();
        db.store_item(conversation).await?;

        let base = Utc::now();
        let mut first = Message::new(
            conversation_id.clone(),
            MessageRole::User,
            "first".to_string(),
            None,
        );
        first.updated_at = base - chrono::Duration::minutes(20);

        let mut second = Message::new(
            conversation_id.clone(),
            MessageRole::AI,
            "second".to_string(),
            None,
        );
        second.updated_at = base - chrono::Duration::minutes(5);

        db.store_item(first).await?;
        db.store_item(second).await?;

        let (_, messages) =
            Conversation::get_complete_conversation(&conversation_id, user_id, &db).await?;

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages.first().expect("expected first message").content,
            "first"
        );
        assert_eq!(
            messages.get(1).expect("expected second message").content,
            "second"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_patch_title_not_found_when_conversation_deleted() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let owner = "owner";
        let conversation = Conversation::new(owner.to_string(), "To delete".to_string());
        let conversation_id = conversation.id.clone();
        db.store_item(conversation).await?;
        db.delete_item::<Conversation>(&conversation_id).await?;

        let result = Conversation::patch_title(&conversation_id, owner, "New title", &db).await;
        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(_)) => {}
            other => anyhow::bail!("expected NotFound, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_conversation_new_initializes_timestamps_and_id() {
        let before = Utc::now();
        let conversation = Conversation::new("user".to_string(), "Title".to_string());
        let after = Utc::now();

        assert!(!conversation.id.is_empty());
        assert!(conversation.created_at >= before && conversation.created_at <= after);
        assert_eq!(conversation.created_at, conversation.updated_at);
        assert_eq!(conversation.user_id, "user");
        assert_eq!(conversation.title, "Title");
    }
}
