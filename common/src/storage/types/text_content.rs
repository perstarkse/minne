use surrealdb::opt::PatchOp;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::file_info::FileInfo;

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Deserialize, Serialize)]
pub struct TextContentSearchResult {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    #[serde(
        serialize_with = "serialize_datetime",
        deserialize_with = "deserialize_datetime",
        default
    )]
    pub created_at: DateTime<Utc>,
    #[serde(
        serialize_with = "serialize_datetime",
        deserialize_with = "deserialize_datetime",
        default
    )]
    pub updated_at: DateTime<Utc>,

    pub text: String,
    #[serde(default)]
    pub file_info: Option<FileInfo>,
    #[serde(default)]
    pub url_info: Option<UrlInfo>,
    #[serde(default)]
    pub context: Option<String>,
    pub category: String,
    pub user_id: String,

    pub score: f32,
    // Highlighted fields from the query aliases
    #[serde(default)]
    pub highlighted_text: Option<String>,
    #[serde(default)]
    pub highlighted_category: Option<String>,
    #[serde(default)]
    pub highlighted_context: Option<String>,
    #[serde(default)]
    pub highlighted_file_name: Option<String>,
    #[serde(default)]
    pub highlighted_url: Option<String>,
    #[serde(default)]
    pub highlighted_url_title: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UrlInfo {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub image_id: String,
}

stored_object!(TextContent, "text_content", {
    text: String,
    file_info: Option<FileInfo>,
    url_info: Option<UrlInfo>,
    context: Option<String>,
    category: String,
    user_id: String
});

impl TextContent {
    pub fn new(
        text: String,
        context: Option<String>,
        category: String,
        file_info: Option<FileInfo>,
        url_info: Option<UrlInfo>,
        user_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            text,
            file_info,
            url_info,
            context,
            category,
            user_id,
        }
    }

    pub async fn patch(
        id: &str,
        context: &str,
        category: &str,
        text: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let now = Utc::now();

        let _res: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/context", context))
            .patch(PatchOp::replace("/category", category))
            .patch(PatchOp::replace("/text", text))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::Datetime::from(now),
            ))
            .await?;

        Ok(())
    }

    pub async fn has_other_with_file(
        file_id: &str,
        exclude_id: &str,
        db: &SurrealDbClient,
    ) -> Result<bool, AppError> {
        let mut response = db
            .client
            .query(
                "SELECT VALUE id FROM type::table($table_name) WHERE file_info.id = $file_id AND id != type::thing($table_name, $exclude_id) LIMIT 1",
            )
            .bind(("table_name", TextContent::table_name()))
            .bind(("file_id", file_id.to_owned()))
            .bind(("exclude_id", exclude_id.to_owned()))
            .await?;

        let existing: Option<surrealdb::sql::Thing> = response.take(0)?;

        Ok(existing.is_some())
    }

    pub async fn search(
        db: &SurrealDbClient,
        search_terms: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<TextContentSearchResult>, AppError> {
        let sql = r#"
            SELECT
                *, 
                search::highlight('<b>', '</b>', 0) AS highlighted_text,
                search::highlight('<b>', '</b>', 1) AS highlighted_category,
                search::highlight('<b>', '</b>', 2) AS highlighted_context,
                search::highlight('<b>', '</b>', 3) AS highlighted_file_name, 
                search::highlight('<b>', '</b>', 4) AS highlighted_url,       
                search::highlight('<b>', '</b>', 5) AS highlighted_url_title, 
                (
                    IF search::score(0) != NONE THEN search::score(0) ELSE 0 END +  
                    IF search::score(1) != NONE THEN search::score(1) ELSE 0 END +  
                    IF search::score(2) != NONE THEN search::score(2) ELSE 0 END +  
                    IF search::score(3) != NONE THEN search::score(3) ELSE 0 END +  
                    IF search::score(4) != NONE THEN search::score(4) ELSE 0 END +  
                    IF search::score(5) != NONE THEN search::score(5) ELSE 0 END    
                ) AS score  
            FROM text_content
            WHERE
                (
                    text @0@ $terms OR
                    category @1@ $terms OR
                    context @2@ $terms OR
                    file_info.file_name @3@ $terms OR
                    url_info.url @4@ $terms OR
                    url_info.title @5@ $terms
                )
                AND user_id = $user_id
            ORDER BY score DESC
            LIMIT $limit;
        "#;

        Ok(db
            .client
            .query(sql)
            .bind(("terms", search_terms.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("limit", limit))
            .await?
            .take(0)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_text_content_creation() {
        // Test basic object creation
        let text = "Test content text".to_string();
        let context = "Test context".to_string();
        let category = "Test category".to_string();
        let user_id = "user123".to_string();

        let text_content = TextContent::new(
            text.clone(),
            Some(context.clone()),
            category.clone(),
            None,
            None,
            user_id.clone(),
        );

        // Check that the fields are set correctly
        assert_eq!(text_content.text, text);
        assert_eq!(text_content.context, Some(context));
        assert_eq!(text_content.category, category);
        assert_eq!(text_content.user_id, user_id);
        assert!(text_content.file_info.is_none());
        assert!(text_content.url_info.is_none());
        assert!(!text_content.id.is_empty());
    }

    #[tokio::test]
    async fn test_text_content_with_url() {
        // Test creating with URL
        let text = "Content with URL".to_string();
        let context = "URL context".to_string();
        let category = "URL category".to_string();
        let user_id = "user123".to_string();
        let title = "page_title".to_string();
        let image_id = "image12312".to_string();
        let url = "https://example.com/document.pdf".to_string();

        let url_info = Some(UrlInfo {
            url,
            title,
            image_id,
        });

        let text_content = TextContent::new(
            text.clone(),
            Some(context.clone()),
            category.clone(),
            None,
            url_info.clone(),
            user_id.clone(),
        );

        // Check URL field is set
        assert_eq!(text_content.url_info, url_info);
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
        let initial_context = "Initial context".to_string();
        let initial_category = "Initial category".to_string();
        let user_id = "user123".to_string();

        let text_content = TextContent::new(
            initial_text,
            Some(initial_context),
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
        let new_context = "Updated context";
        let new_category = "Updated category";
        let new_text = "Updated text content";

        // Apply the patch
        TextContent::patch(&text_content.id, new_context, new_category, new_text, &db)
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
        assert_eq!(updated_content.context, Some(new_context.to_string()));
        assert_eq!(updated_content.category, new_category);
        assert_eq!(updated_content.text, new_text);
        assert!(updated_content.updated_at > text_content.updated_at);
    }

    #[tokio::test]
    async fn test_has_other_with_file_detects_shared_usage() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let user_id = "user123".to_string();
        let file_info = FileInfo {
            id: "file-1".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            sha256: "sha-test".to_string(),
            path: "user123/file-1/test.txt".to_string(),
            file_name: "test.txt".to_string(),
            mime_type: "text/plain".to_string(),
            user_id: user_id.clone(),
        };

        let content_a = TextContent::new(
            "First".to_string(),
            Some("ctx-a".to_string()),
            "category".to_string(),
            Some(file_info.clone()),
            None,
            user_id.clone(),
        );
        let content_b = TextContent::new(
            "Second".to_string(),
            Some("ctx-b".to_string()),
            "category".to_string(),
            Some(file_info.clone()),
            None,
            user_id.clone(),
        );

        db.store_item(content_a.clone())
            .await
            .expect("Failed to store first content");
        db.store_item(content_b.clone())
            .await
            .expect("Failed to store second content");

        let has_other = TextContent::has_other_with_file(&file_info.id, &content_a.id, &db)
            .await
            .expect("Failed to check for shared file usage");
        assert!(has_other);

        let _removed: Option<TextContent> = db
            .delete_item(&content_b.id)
            .await
            .expect("Failed to delete second content");

        let has_other_after = TextContent::has_other_with_file(&file_info.id, &content_a.id, &db)
            .await
            .expect("Failed to check shared usage after delete");
        assert!(!has_other_after);
    }
}
