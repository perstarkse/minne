use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use surrealdb::opt::PatchOp;
use surrealdb::RecordId;
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
    #[must_use]
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

    /// SurrealQL deletes for ingested child rows keyed by `source_id` (no transaction wrapper).
    ///
    /// Used inside larger transactions (e.g. ingestion `persist_artifacts`) and mirrored by
    /// [`Self::clear_ingested_children`].
    pub const CLEAR_INGESTED_CHILD_ROWS_SURQL: &'static str = r"
DELETE relates_to WHERE metadata.source_id = $source_id AND metadata.user_id = $user_id;
DELETE text_chunk_embedding WHERE source_id = $source_id;
DELETE text_chunk WHERE source_id = $source_id;
DELETE knowledge_entity_embedding WHERE source_id = $source_id;
DELETE knowledge_entity WHERE source_id = $source_id;
";

    /// Removes chunks, embeddings, entities, and relationships for one ingested document snapshot.
    pub async fn clear_ingested_children(
        source_id: &str,
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "BEGIN TRANSACTION;\n{} COMMIT TRANSACTION;",
            Self::CLEAR_INGESTED_CHILD_ROWS_SURQL
        );

        db.client
            .query(query)
            .bind(("source_id", source_id.to_string()))
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        Ok(())
    }

    pub async fn patch(
        id: &str,
        context: &str,
        category: &str,
        text: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let now = Utc::now();

        let updated: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/context", context))
            .patch(PatchOp::replace("/category", category))
            .patch(PatchOp::replace("/text", text))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::Datetime::from(now),
            ))
            .await
            .map_err(AppError::from)?;

        if updated.is_none() {
            return Err(AppError::NotFound(format!("text content {id} not found")));
        }

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
            .await
            .map_err(AppError::from)?;

        let existing: Option<surrealdb::sql::Thing> = response.take(0).map_err(AppError::from)?;

        Ok(existing.is_some())
    }

    pub async fn search(
        db: &SurrealDbClient,
        search_terms: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<TextContentSearchResult>, AppError> {
        let sql = format!(
            r#"
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
            FROM {table}
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
            "#,
            table = Self::table_name(),
        );

        db.client
            .query(sql)
            .bind(("terms", search_terms.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("limit", limit))
            .await
            .map_err(AppError::from)?
            .take(0)
            .map_err(AppError::from)
    }

    /// Builds a fallback display label for a source id when no matching content row exists.
    #[must_use]
    pub fn fallback_source_label(source_id: &str) -> String {
        format!("Text snippet: {}", source_id_suffix(source_id))
    }

    /// Resolves human-readable labels for the given source ids owned by `user_id`.
    pub async fn resolve_source_labels(
        db: &SurrealDbClient,
        user_id: &str,
        source_ids: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<HashMap<String, String>, AppError> {
        let source_ids: HashSet<String> = source_ids
            .into_iter()
            .map(|id| id.as_ref().to_string())
            .collect();

        if source_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let record_ids: Vec<RecordId> = source_ids
            .iter()
            .filter_map(|id| {
                if id.contains(':') {
                    RecordId::from_str(id).ok()
                } else {
                    Some(RecordId::from_table_key(Self::table_name(), id))
                }
            })
            .collect();

        let mut response = db
            .client
            .query(
                "SELECT id, url_info, file_info, context, category, text FROM type::table($table_name) WHERE user_id = $user_id AND id INSIDE $record_ids",
            )
            .bind(("table_name", Self::table_name()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("record_ids", record_ids))
            .await
            .map_err(AppError::from)?;

        let contents: Vec<SourceLabelRow> = response.take(0).map_err(AppError::from)?;

        tracing::debug!(
            source_id_count = source_ids.len(),
            label_row_count = contents.len(),
            "resolved source labels"
        );

        let mut labels = HashMap::new();
        for content in contents {
            let label = build_source_label(&content);
            labels.insert(content.id.clone(), label.clone());
            labels.insert(format!("{}:{}", Self::table_name(), content.id), label);
        }

        Ok(labels)
    }
}

const SOURCE_LABEL_MAX_CHARS: usize = 80;

#[derive(Deserialize)]
struct SourceLabelRow {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    id: String,
    #[serde(default)]
    url_info: Option<UrlInfo>,
    #[serde(default)]
    file_info: Option<FileInfo>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    category: String,
    #[serde(default)]
    text: String,
}

fn source_id_suffix(source_id: &str) -> String {
    let start = source_id.len().saturating_sub(8);
    source_id[start..].to_string()
}

fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    const ELLIPSIS: &str = "…";

    if max_chars == 0 {
        return if value.is_empty() {
            String::new()
        } else {
            ELLIPSIS.to_string()
        };
    }

    let mut end_byte = value.len();
    for (count, (idx, _)) in value.char_indices().enumerate() {
        if count == max_chars {
            end_byte = idx;
            break;
        }
    }

    if end_byte == value.len() {
        return value.to_string();
    }

    format!("{}{}", &value[..end_byte], ELLIPSIS)
}

fn first_non_empty_line(text: &str, max_chars: usize) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(truncate_with_ellipsis(trimmed, max_chars))
        }
    })
}

fn build_source_label(row: &SourceLabelRow) -> String {
    if let Some(url_info) = row.url_info.as_ref() {
        let title = url_info.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }

        let url = url_info.url.trim();
        if !url.is_empty() {
            return url.to_string();
        }
    }

    if let Some(file_info) = row.file_info.as_ref() {
        let name = file_info.file_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }

    if let Some(context) = row.context.as_ref() {
        let trimmed = context.trim();
        if !trimmed.is_empty() {
            return truncate_with_ellipsis(trimmed, SOURCE_LABEL_MAX_CHARS);
        }
    }

    if let Some(text_label) = first_non_empty_line(&row.text, SOURCE_LABEL_MAX_CHARS) {
        return text_label;
    }

    let category = row.category.trim();
    if !category.is_empty() {
        return truncate_with_ellipsis(category, SOURCE_LABEL_MAX_CHARS);
    }

    TextContent::fallback_source_label(&row.id)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use anyhow::{self, Context};

    use super::*;
    use crate::{
        storage::types::{
            knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
            knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk,
        },
        test_utils::{setup_test_db, setup_test_db_with_runtime_indexes},
    };

    #[tokio::test]
    async fn test_text_content_creation() -> anyhow::Result<()> {
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
        Ok(())
    }

    #[tokio::test]
    async fn test_text_content_with_url() -> anyhow::Result<()> {
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
            text,
            Some(context),
            category,
            None,
            url_info.clone(),
            user_id,
        );

        // Check URL field is set
        assert_eq!(text_content.url_info, url_info);
        Ok(())
    }

    #[tokio::test]
    async fn test_text_content_patch() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

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
            .with_context(|| "Failed to store text content".to_string())?;
        assert!(stored.is_some());

        // New values for patch
        let new_context = "Updated context";
        let new_category = "Updated category";
        let new_text = "Updated text content";

        // Apply the patch
        TextContent::patch(&text_content.id, new_context, new_category, new_text, &db)
            .await
            .with_context(|| "Failed to patch text content".to_string())?;

        // Retrieve the updated content
        let updated: Option<TextContent> = db
            .get_item(&text_content.id)
            .await
            .with_context(|| "Failed to get updated text content".to_string())?;
        let updated_content = updated.with_context(|| "expected updated content".to_string())?;

        // Verify the updates
        assert_eq!(updated_content.context, Some(new_context.to_string()));
        assert_eq!(updated_content.category, new_category);
        assert_eq!(updated_content.text, new_text);
        assert!(updated_content.updated_at > text_content.updated_at);
        Ok(())
    }

    #[tokio::test]
    async fn test_text_content_patch_not_found() -> anyhow::Result<()> {
        let db = setup_test_db_with_runtime_indexes().await?;

        let err = TextContent::patch("missing-id", "ctx", "cat", "text", &db)
            .await
            .expect_err("expected not found");

        assert!(matches!(err, AppError::NotFound(_)));
        Ok(())
    }

    #[tokio::test]
    async fn test_has_other_with_file_detects_shared_usage() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

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
            .with_context(|| "Failed to store first content".to_string())?;
        db.store_item(content_b.clone())
            .await
            .with_context(|| "Failed to store second content".to_string())?;

        let has_other = TextContent::has_other_with_file(&file_info.id, &content_a.id, &db)
            .await
            .with_context(|| "Failed to check for shared file usage".to_string())?;
        assert!(has_other);

        let _removed: Option<TextContent> = db
            .delete_item(&content_b.id)
            .await
            .with_context(|| "Failed to delete second content".to_string())?;

        let has_other_after = TextContent::has_other_with_file(&file_info.id, &content_a.id, &db)
            .await
            .with_context(|| "Failed to check shared usage after delete".to_string())?;
        assert!(!has_other_after);
        Ok(())
    }

    #[tokio::test]
    async fn test_search_returns_empty_when_no_content() -> anyhow::Result<()> {
        let db = setup_test_db_with_runtime_indexes().await?;

        let results = TextContent::search(&db, "hello", "user", 5)
            .await
            .with_context(|| "search".to_string())?;

        assert!(results.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_search_finds_matching_text_and_filters_user() -> anyhow::Result<()> {
        let db = setup_test_db_with_runtime_indexes().await?;
        let user_id = "search_user";

        let matching = TextContent::new(
            "rust programming language".to_string(),
            Some("context".to_string()),
            "notes".to_string(),
            None,
            None,
            user_id.to_string(),
        );
        let other_user = TextContent::new(
            "rust programming language".to_string(),
            None,
            "notes".to_string(),
            None,
            None,
            "other_user".to_string(),
        );

        db.store_item(matching.clone())
            .await
            .with_context(|| "store matching".to_string())?;
        db.store_item(other_user)
            .await
            .with_context(|| "store other user".to_string())?;

        let results = TextContent::search(&db, "rust", user_id, 5)
            .await
            .with_context(|| "search".to_string())?;

        assert_eq!(results.len(), 1);
        let row = results.first().context("expected one result")?;
        assert_eq!(row.id, matching.id);
        assert_eq!(row.user_id, user_id);
        assert!(row.score.is_finite());
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_source_labels_uses_url_title() -> anyhow::Result<()> {
        let db = setup_test_db_with_runtime_indexes().await?;
        let user_id = "label_user";

        let content = TextContent::new(
            "body".to_string(),
            None,
            "notes".to_string(),
            None,
            Some(UrlInfo {
                url: "https://example.com/doc".to_string(),
                title: "Example Document".to_string(),
                image_id: String::new(),
            }),
            user_id.to_string(),
        );
        db.store_item(content.clone()).await?;

        let labels = TextContent::resolve_source_labels(&db, user_id, [content.id.clone()]).await?;

        assert_eq!(
            labels.get(&content.id),
            Some(&"Example Document".to_string())
        );
        assert_eq!(
            labels.get(&format!("text_content:{}", content.id)),
            Some(&"Example Document".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn clear_ingested_children_removes_chunks_entities_and_relationships(
    ) -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "clear-user";
        let source_id = Uuid::new_v4().to_string();

        let entity_a = KnowledgeEntity::new(
            source_id.clone(),
            "entity-a".to_string(),
            "desc-a".to_string(),
            KnowledgeEntityType::Idea,
            None,
            user_id.to_string(),
        );
        let entity_b = KnowledgeEntity::new(
            source_id.clone(),
            "entity-b".to_string(),
            "desc-b".to_string(),
            KnowledgeEntityType::Idea,
            None,
            user_id.to_string(),
        );
        KnowledgeEntity::store_with_embedding(entity_a.clone(), vec![0.1; 3], 3, &db)
            .await
            .context("store entity a")?;
        KnowledgeEntity::store_with_embedding(entity_b.clone(), vec![0.2; 3], 3, &db)
            .await
            .context("store entity b")?;

        let chunk = TextChunk::new(source_id.clone(), "chunk".to_string(), user_id.to_string());
        TextChunk::store_with_embedding(chunk, vec![0.3; 3], 3, &db)
            .await
            .context("store chunk")?;

        KnowledgeRelationship::new(
            entity_a.id.clone(),
            entity_b.id,
            user_id.to_string(),
            source_id.clone(),
            "relates_to".to_string(),
        )
        .store_relationship(&db)
        .await
        .context("store relationship")?;

        TextContent::clear_ingested_children(&source_id, user_id, &db)
            .await
            .context("clear ingested children")?;

        let chunks: Vec<TextChunk> = db
            .client
            .query("SELECT * FROM text_chunk WHERE source_id = $source_id;")
            .bind(("source_id", source_id.clone()))
            .await?
            .take(0)?;
        assert!(chunks.is_empty());

        let entities: Vec<KnowledgeEntity> = db
            .client
            .query("SELECT * FROM knowledge_entity WHERE source_id = $source_id;")
            .bind(("source_id", source_id.clone()))
            .await?
            .take(0)?;
        assert!(entities.is_empty());

        let relationships: Vec<KnowledgeRelationship> = db
            .client
            .query("SELECT * FROM relates_to WHERE metadata.source_id = $source_id;")
            .bind(("source_id", source_id))
            .await?
            .take(0)?;
        assert!(relationships.is_empty());

        Ok(())
    }
}
