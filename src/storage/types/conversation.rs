use uuid::Uuid;

use crate::stored_object;

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
}
