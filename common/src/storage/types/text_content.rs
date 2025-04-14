use surrealdb::opt::PatchOp;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::file_info::FileInfo;

stored_object!(TextContent, "text_content", {
    text: String,
    file_info: Option<FileInfo>,
    url: Option<String>,
    instructions: String,
    category: String,
    user_id: String
});

impl TextContent {
    pub fn new(
        text: String,
        instructions: String,
        category: String,
        file_info: Option<FileInfo>,
        url: Option<String>,
        user_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            text,
            file_info,
            url,
            instructions,
            category,
            user_id,
        }
    }

    pub async fn patch(
        id: &str,
        instructions: &str,
        category: &str,
        text: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let now = Utc::now();

        let _res: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/instructions", instructions))
            .patch(PatchOp::replace("/category", category))
            .patch(PatchOp::replace("/text", text))
            .patch(PatchOp::replace("/updated_at", now))
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_text_content_creation() {
        // Test basic object creation
        let text = "Test content text".to_string();
        let instructions = "Test instructions".to_string();
        let category = "Test category".to_string();
        let user_id = "user123".to_string();

        let text_content = TextContent::new(
            text.clone(),
            instructions.clone(),
            category.clone(),
            None,
            None,
            user_id.clone(),
        );

        // Check that the fields are set correctly
        assert_eq!(text_content.text, text);
        assert_eq!(text_content.instructions, instructions);
        assert_eq!(text_content.category, category);
        assert_eq!(text_content.user_id, user_id);
        assert!(text_content.file_info.is_none());
        assert!(text_content.url.is_none());
        assert!(!text_content.id.is_empty());
    }

    #[tokio::test]
    async fn test_text_content_with_url() {
        // Test creating with URL
        let text = "Content with URL".to_string();
        let instructions = "URL instructions".to_string();
        let category = "URL category".to_string();
        let user_id = "user123".to_string();
        let url = Some("https://example.com/document.pdf".to_string());

        let text_content = TextContent::new(
            text.clone(),
            instructions.clone(),
            category.clone(),
            None,
            url.clone(),
            user_id.clone(),
        );

        // Check URL field is set
        assert_eq!(text_content.url, url);
    }

    #[tokio::test]
    async fn test_text_content_patch() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create initial text content
        let initial_text = "Initial text".to_string();
        let initial_instructions = "Initial instructions".to_string();
        let initial_category = "Initial category".to_string();
        let user_id = "user123".to_string();

        let text_content = TextContent::new(
            initial_text,
            initial_instructions,
            initial_category,
            None,
            None,
            user_id,
        );

        // Store the text content
        let stored: Option<TextContent> = db
            .store_item(text_content.clone())
            .await
            .expect("Failed to store text content");
        assert!(stored.is_some());

        // New values for patch
        let new_instructions = "Updated instructions";
        let new_category = "Updated category";
        let new_text = "Updated text content";

        // Apply the patch
        TextContent::patch(
            &text_content.id,
            new_instructions,
            new_category,
            new_text,
            &db,
        )
        .await
        .expect("Failed to patch text content");

        // Retrieve the updated content
        let updated: Option<TextContent> = db
            .get_item(&text_content.id)
            .await
            .expect("Failed to get updated text content");
        assert!(updated.is_some());

        let updated_content = updated.unwrap();

        // Verify the updates
        assert_eq!(updated_content.instructions, new_instructions);
        assert_eq!(updated_content.category, new_category);
        assert_eq!(updated_content.text, new_text);
        assert!(updated_content.updated_at > text_content.updated_at);
    }
}
