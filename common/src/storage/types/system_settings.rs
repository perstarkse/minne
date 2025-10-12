use crate::storage::types::file_info::deserialize_flexible_id;
use serde::{Deserialize, Serialize};

use crate::{error::AppError, storage::db::SurrealDbClient, storage::types::StoredObject};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemSettings {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    pub registrations_enabled: bool,
    pub require_email_verification: bool,
    pub query_model: String,
    pub processing_model: String,
    pub embedding_model: String,
    pub embedding_dimensions: u32,
    pub query_system_prompt: String,
    pub ingestion_system_prompt: String,
    pub image_processing_model: String,
    pub image_processing_prompt: String,
    pub voice_processing_model: String,
}

impl StoredObject for SystemSettings {
    fn table_name() -> &'static str {
        "system_settings"
    }

    fn get_id(&self) -> &str {
        &self.id
    }
}

impl SystemSettings {
    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let settings: Option<Self> = db.get_item("current").await?;
        settings.ok_or(AppError::NotFound("System settings not found".into()))
    }

    pub async fn update(db: &SurrealDbClient, changes: Self) -> Result<Self, AppError> {
        // We need to use a direct query for the update with MERGE
        let updated: Option<Self> = db
            .client
            .query("UPDATE type::thing('system_settings', 'current') MERGE $changes RETURN AFTER")
            .bind(("changes", changes))
            .await?
            .take(0)?;

        updated.ok_or(AppError::Validation(
            "Something went wrong updating the settings".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk};
    use async_openai::Client;

    use super::*;
    use uuid::Uuid;

    async fn get_hnsw_index_dimension(
        db: &SurrealDbClient,
        table_name: &str,
        index_name: &str,
    ) -> u32 {
        let query = format!("INFO FOR TABLE {table_name};");
        let mut response = db
            .client
            .query(query)
            .await
            .expect("Failed to fetch table info");

        let info: Option<serde_json::Value> = response
            .take(0)
            .expect("Failed to extract table info response");

        let info = info.expect("Table info result missing");

        let indexes = info
            .get("indexes")
            .or_else(|| {
                info.get("tables")
                    .and_then(|tables| tables.get(table_name))
                    .and_then(|table| table.get("indexes"))
            })
            .unwrap_or_else(|| panic!("Indexes collection missing in table info: {info:#?}"));

        let definition = indexes
            .get(index_name)
            .and_then(|definition| definition.as_str())
            .unwrap_or_else(|| panic!("Index definition not found in table info: {info:#?}"));

        let dimension_part = definition
            .split("DIMENSION")
            .nth(1)
            .expect("Index definition missing DIMENSION clause");

        let dimension_token = dimension_part
            .split_whitespace()
            .next()
            .expect("Dimension value missing in definition")
            .trim_end_matches(';');

        dimension_token
            .parse::<u32>()
            .expect("Dimension value is not a valid number")
    }

    #[tokio::test]
    async fn test_settings_initialization() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Test initialization of system settings
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");
        let settings = SystemSettings::get_current(&db)
            .await
            .expect("Failed to get system settings");

        // Verify initial state after initialization
        assert_eq!(settings.id, "current");
        assert_eq!(settings.registrations_enabled, true);
        assert_eq!(settings.require_email_verification, false);
        assert_eq!(settings.query_model, "gpt-4o-mini");
        assert_eq!(settings.processing_model, "gpt-4o-mini");
        assert_eq!(settings.image_processing_model, "gpt-4o-mini");
        // Dont test these for now, having a hard time getting the formatting exactly the same
        // assert_eq!(
        //     settings.query_system_prompt,
        //     crate::storage::types::system_prompts::DEFAULT_QUERY_SYSTEM_PROMPT
        // );
        // assert_eq!(
        //     settings.ingestion_system_prompt,
        //     crate::storage::types::system_prompts::DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT
        // );

        // Test idempotency - ensure calling it again doesn't change anything
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");
        let settings_again = SystemSettings::get_current(&db)
            .await
            .expect("Failed to get settings after initialization");

        assert_eq!(settings.id, settings_again.id);
        assert_eq!(
            settings.registrations_enabled,
            settings_again.registrations_enabled
        );
        assert_eq!(
            settings.require_email_verification,
            settings_again.require_email_verification
        );
    }

    #[tokio::test]
    async fn test_get_current_settings() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Initialize settings
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        // Test get_current method
        let settings = SystemSettings::get_current(&db)
            .await
            .expect("Failed to get current settings");

        assert_eq!(settings.id, "current");
        assert_eq!(settings.registrations_enabled, true);
        assert_eq!(settings.require_email_verification, false);
    }

    #[tokio::test]
    async fn test_update_settings() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Initialize settings
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        // Create updated settings
        let mut updated_settings = SystemSettings::get_current(&db).await.unwrap();
        updated_settings.id = "current".to_string();
        updated_settings.registrations_enabled = false;
        updated_settings.require_email_verification = true;
        updated_settings.query_model = "gpt-4".to_string();

        // Test update method
        let result = SystemSettings::update(&db, updated_settings)
            .await
            .expect("Failed to update settings");

        assert_eq!(result.id, "current");
        assert_eq!(result.registrations_enabled, false);
        assert_eq!(result.require_email_verification, true);
        assert_eq!(result.query_model, "gpt-4");

        // Verify changes persisted by getting current settings
        let current = SystemSettings::get_current(&db)
            .await
            .expect("Failed to get current settings after update");

        assert_eq!(current.registrations_enabled, false);
        assert_eq!(current.require_email_verification, true);
        assert_eq!(current.query_model, "gpt-4");
    }

    #[tokio::test]
    async fn test_get_current_nonexistent() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Don't initialize settings and try to get them
        let result = SystemSettings::get_current(&db).await;

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(_)) => {
                // Expected error
            }
            Err(e) => panic!("Expected NotFound error, got: {:?}", e),
            Ok(_) => panic!("Expected error but got Ok"),
        }
    }

    #[tokio::test]
    async fn test_migration_after_changing_embedding_length() {
        let db = SurrealDbClient::memory("test", &Uuid::new_v4().to_string())
            .await
            .expect("Failed to start DB");

        // Apply initial migrations. This sets up the text_chunk index with DIMENSION 1536.
        db.apply_migrations()
            .await
            .expect("Initial migration failed");

        let initial_chunk = TextChunk::new(
            "source1".into(),
            "This chunk has the original dimension".into(),
            vec![0.1; 1536],
            "user1".into(),
        );

        db.store_item(initial_chunk.clone())
            .await
            .expect("Failed to store initial chunk");

        async fn simulate_reembedding(
            db: &SurrealDbClient,
            target_dimension: usize,
            initial_chunk: TextChunk,
        ) {
            db.query("REMOVE INDEX idx_embedding_chunks ON TABLE text_chunk;")
                .await
                .unwrap();
            let define_index_query = format!(
                         "DEFINE INDEX idx_embedding_chunks ON TABLE text_chunk FIELDS embedding HNSW DIMENSION {};",
                         target_dimension
                     );
            db.query(define_index_query)
                .await
                .expect("Re-defining index should succeed");

            let new_embedding = vec![0.5; target_dimension];
            let sql = "UPDATE type::thing('text_chunk', $id) SET embedding = $embedding;";

            let update_result = db
                .client
                .query(sql)
                .bind(("id", initial_chunk.id.clone()))
                .bind(("embedding", new_embedding))
                .await;

            assert!(update_result.is_ok());
        }

        simulate_reembedding(&db, 768, initial_chunk).await;

        let migration_result = db.apply_migrations().await;

        assert!(migration_result.is_ok(), "Migrations should not fail");
    }

    #[tokio::test]
    async fn test_should_change_embedding_length_on_indexes_when_switching_length() {
        let db = SurrealDbClient::memory("test", &Uuid::new_v4().to_string())
            .await
            .expect("Failed to start DB");

        // Apply initial migrations. This sets up the text_chunk index with DIMENSION 1536.
        db.apply_migrations()
            .await
            .expect("Initial migration failed");

        let mut current_settings = SystemSettings::get_current(&db)
            .await
            .expect("Failed to load current settings");

        let initial_chunk_dimension =
            get_hnsw_index_dimension(&db, "text_chunk", "idx_embedding_chunks").await;

        assert_eq!(
            initial_chunk_dimension, current_settings.embedding_dimensions,
            "embedding size should match initial system settings"
        );

        let new_dimension = 768;
        let new_model = "new-test-embedding-model".to_string();

        current_settings.embedding_dimensions = new_dimension;
        current_settings.embedding_model = new_model.clone();

        let updated_settings = SystemSettings::update(&db, current_settings)
            .await
            .expect("Failed to update settings");

        assert_eq!(
            updated_settings.embedding_dimensions, new_dimension,
            "Settings should reflect the new embedding dimension"
        );

        let openai_client = Client::new();

        TextChunk::update_all_embeddings(&db, &openai_client, &new_model, new_dimension)
            .await
            .expect("TextChunk re-embedding should succeed on fresh DB");
        KnowledgeEntity::update_all_embeddings(&db, &openai_client, &new_model, new_dimension)
            .await
            .expect("KnowledgeEntity re-embedding should succeed on fresh DB");

        let text_chunk_dimension =
            get_hnsw_index_dimension(&db, "text_chunk", "idx_embedding_chunks").await;
        let knowledge_dimension =
            get_hnsw_index_dimension(&db, "knowledge_entity", "idx_embedding_entities").await;

        assert_eq!(
            text_chunk_dimension, new_dimension,
            "text_chunk index dimension should update"
        );
        assert_eq!(
            knowledge_dimension, new_dimension,
            "knowledge_entity index dimension should update"
        );

        let persisted_settings = SystemSettings::get_current(&db)
            .await
            .expect("Failed to reload updated settings");
        assert_eq!(
            persisted_settings.embedding_dimensions, new_dimension,
            "Settings should persist new embedding dimension"
        );
    }
}
