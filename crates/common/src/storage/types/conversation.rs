use uuid::Uuid;

use crate::{
    error::AppError,
    storage::db::{get_item, SurrealDbClient},
    stored_object,
};

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
        let conversation: Conversation = get_item(&db, conversation_id)
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
}
