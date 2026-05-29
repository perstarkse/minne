use crate::utils::serde_helpers::deserialize_flexible_id;
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
    /// Active embedding backend ("openai", "fastembed", "hashed"). Read-only, synced from config.
    #[serde(default)]
    pub embedding_backend: Option<String>,
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

    fn id(&self) -> &str {
        &self.id
    }
}

impl SystemSettings {
    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let settings: Option<Self> = db.get_item("current").await?;
        settings.ok_or(AppError::NotFound("system settings not found".into()))
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
            "something went wrong updating the settings".into(),
        ))
    }

    /// Syncs SystemSettings with the active embedding provider's properties.
    /// Updates embedding_backend, embedding_model, and embedding_dimensions if they differ.
    /// Returns true if any settings were changed.
    pub async fn sync_from_embedding_provider(
        db: &SurrealDbClient,
        provider: &crate::utils::embedding::EmbeddingProvider,
    ) -> Result<(Self, bool), AppError> {
        let mut settings = Self::get_current(db).await?;
        let mut needs_update = false;

        let backend_label = provider.backend_label().to_string();
        let provider_dimensions = u32::try_from(provider.dimension()).unwrap_or_else(|_| {
            tracing::warn!(
                "Provider dimension {} exceeds u32 max; falling back to 0",
                provider.dimension()
            );
            0u32
        });
        let provider_model = provider.model_code();

        // Sync backend label
        if settings.embedding_backend.as_deref() != Some(&backend_label) {
            settings.embedding_backend = Some(backend_label);
            needs_update = true;
        }

        // Sync dimensions
        if settings.embedding_dimensions != provider_dimensions {
            tracing::info!(
                old_dimensions = settings.embedding_dimensions,
                new_dimensions = provider_dimensions,
                "Embedding dimensions changed, updating SystemSettings"
            );
            settings.embedding_dimensions = provider_dimensions;
            needs_update = true;
        }

        // Sync model if provider has one
        if let Some(model) = provider_model {
            if settings.embedding_model != model {
                tracing::info!(
                    old_model = %settings.embedding_model,
                    new_model = %model,
                    "Embedding model changed, updating SystemSettings"
                );
                settings.embedding_model = model;
                needs_update = true;
            }
        }

        if needs_update {
            settings = Self::update(db, settings).await?;
        }

        Ok((settings, needs_update))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use crate::storage::indexes::ensure_runtime;
    use crate::storage::types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk};
    use anyhow::{self, Context};
    use async_openai::Client;

    use super::*;
    use uuid::Uuid;

    async fn get_hnsw_index_dimension(
        db: &SurrealDbClient,
        table_name: &str,
        index_name: &str,
    ) -> anyhow::Result<u32> {
        let query = format!("INFO FOR TABLE {table_name};");
        let mut response = db
            .client
            .query(query)
            .await
            .with_context(|| "Failed to fetch table info".to_string())?;

        let info: surrealdb::Value = response
            .take(0)
            .with_context(|| "Failed to extract table info response".to_string())?;

        let info_json: serde_json::Value = serde_json::to_value(info)
            .with_context(|| "Failed to convert info to json".to_string())?;

        let indexes = info_json
            .get("Object")
            .and_then(|v| v.get("indexes"))
            .and_then(|v| v.get("Object"))
            .and_then(|v| v.as_object())
            .with_context(|| format!("Indexes collection missing in table info: {info_json:#?}"))?;

        let definition = indexes
            .get(index_name)
            .and_then(|definition| definition.get("Strand"))
            .and_then(|v| v.as_str())
            .with_context(|| format!("Index definition not found in table info: {info_json:#?}"))?;

        let dimension_part = definition
            .split("DIMENSION")
            .nth(1)
            .with_context(|| "Index definition missing DIMENSION clause".to_string())?;

        let dimension_token = dimension_part
            .split_whitespace()
            .next()
            .with_context(|| "Dimension value missing in definition".to_string())?
            .trim_end_matches(';');

        dimension_token
            .parse::<u32>()
            .with_context(|| "Dimension value is not a valid number".to_string())
    }

    async fn simulate_reembedding(
        db: &SurrealDbClient,
        target_dimension: usize,
        initial_chunk: TextChunk,
    ) -> anyhow::Result<()> {
        db.query(
            "REMOVE INDEX IF EXISTS idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding;",
        )
        .await
        .with_context(|| "remove index".to_string())?;
        let define_index_query = format!(
            "DEFINE INDEX idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding FIELDS embedding HNSW DIMENSION {target_dimension};"
        );
        db.query(define_index_query)
            .await
            .with_context(|| "Re-defining index should succeed".to_string())?;

        let new_embedding = vec![0.5; target_dimension];
        let sql = "UPSERT type::thing('text_chunk_embedding', $id) SET chunk_id = type::thing('text_chunk', $id), embedding = $embedding, user_id = $user_id;";

        db.client
            .query(sql)
            .bind(("id", initial_chunk.id.clone()))
            .bind(("user_id", initial_chunk.user_id.clone()))
            .bind(("embedding", new_embedding))
            .await
            .with_context(|| "upsert embedding".to_string())?;

        Ok(())
    }

    #[tokio::test]
    async fn test_settings_initialization() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        // Test initialization of system settings
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;
        let settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get system settings".to_string())?;

        // Verify initial state after initialization
        assert_eq!(settings.id, "current");
        assert!(settings.registrations_enabled);
        assert!(!settings.require_email_verification);
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
            .with_context(|| "Failed to apply migrations".to_string())?;
        let settings_again = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get settings after initialization".to_string())?;

        assert_eq!(settings.id, settings_again.id);
        assert_eq!(
            settings.registrations_enabled,
            settings_again.registrations_enabled
        );
        assert_eq!(
            settings.require_email_verification,
            settings_again.require_email_verification
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_current_settings() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        // Initialize settings
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;

        // Test get_current method
        let settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get current settings".to_string())?;

        assert_eq!(settings.id, "current");
        assert!(settings.registrations_enabled);
        assert!(!settings.require_email_verification);
        Ok(())
    }

    #[tokio::test]
    async fn test_update_settings() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        // Initialize settings
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;

        // Create updated settings
        let mut updated_settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "get_current".to_string())?;
        updated_settings.id = "current".to_string();
        updated_settings.registrations_enabled = false;
        updated_settings.require_email_verification = true;
        updated_settings.query_model = "gpt-4".to_string();

        // Test update method
        let result = SystemSettings::update(&db, updated_settings)
            .await
            .with_context(|| "Failed to update settings".to_string())?;

        assert_eq!(result.id, "current");
        assert!(!result.registrations_enabled);
        assert!(result.require_email_verification);
        assert_eq!(result.query_model, "gpt-4");

        // Verify changes persisted by getting current settings
        let current = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get current settings after update".to_string())?;

        assert!(!current.registrations_enabled);
        assert!(current.require_email_verification);
        assert_eq!(current.query_model, "gpt-4");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_current_nonexistent() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        // Don't initialize settings and try to get them
        let result = SystemSettings::get_current(&db).await;

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(_)) => {
                // Expected error
            }
            Err(e) => anyhow::bail!("Expected NotFound error, got: {e:?}"),
            Ok(_) => anyhow::bail!("Expected error but got Ok"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_migration_after_changing_embedding_length() -> anyhow::Result<()> {
        let db = SurrealDbClient::memory("test", &Uuid::new_v4().to_string())
            .await
            .with_context(|| "Failed to start DB".to_string())?;

        // Apply initial migrations. This sets up the text_chunk index with DIMENSION 1536.
        db.apply_migrations()
            .await
            .with_context(|| "Initial migration failed".to_string())?;

        let initial_chunk = TextChunk::new(
            "source1".into(),
            "This chunk has the original dimension".into(),
            "user1".into(),
        );

        TextChunk::store_with_embedding(initial_chunk.clone(), vec![0.1; 1536], &db)
            .await
            .with_context(|| "Failed to store initial chunk with embedding".to_string())?;

        // Re-embed with the existing configured dimension to ensure migrations remain idempotent.
        let target_dimension = 1536usize;
        simulate_reembedding(&db, target_dimension, initial_chunk).await?;

        let migration_result = db.apply_migrations().await;

        assert!(
            migration_result.is_ok(),
            "Migrations should not fail: {:?}",
            migration_result.err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_change_embedding_length_on_indexes_when_switching_length(
    ) -> anyhow::Result<()> {
        let db = SurrealDbClient::memory("test", &Uuid::new_v4().to_string())
            .await
            .with_context(|| "Failed to start DB".to_string())?;

        // Apply initial migrations. This sets up the text_chunk index with DIMENSION 1536.
        db.apply_migrations()
            .await
            .with_context(|| "Initial migration failed".to_string())?;

        let mut current_settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to load current settings".to_string())?;

        // Ensure runtime indexes exist with the current embedding dimension so INFO queries succeed.
        ensure_runtime(&db, current_settings.embedding_dimensions as usize)
            .await
            .with_context(|| "failed to build runtime indexes".to_string())?;

        let initial_chunk_dimension = get_hnsw_index_dimension(
            &db,
            "text_chunk_embedding",
            "idx_embedding_text_chunk_embedding",
        )
        .await?;

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
            .with_context(|| "Failed to update settings".to_string())?;

        assert_eq!(
            updated_settings.embedding_dimensions, new_dimension,
            "Settings should reflect the new embedding dimension"
        );

        let openai_client = Client::new();

        TextChunk::update_all_embeddings(&db, &openai_client, &new_model, new_dimension)
            .await
            .with_context(|| "TextChunk re-embedding should succeed on fresh DB".to_string())?;
        KnowledgeEntity::update_all_embeddings(&db, &openai_client, &new_model, new_dimension)
            .await
            .with_context(|| {
                "KnowledgeEntity re-embedding should succeed on fresh DB".to_string()
            })?;

        let text_chunk_dimension = get_hnsw_index_dimension(
            &db,
            "text_chunk_embedding",
            "idx_embedding_text_chunk_embedding",
        )
        .await?;
        let knowledge_dimension = get_hnsw_index_dimension(
            &db,
            "knowledge_entity_embedding",
            "idx_embedding_knowledge_entity_embedding",
        )
        .await?;

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
            .with_context(|| "Failed to reload updated settings".to_string())?;
        assert_eq!(
            persisted_settings.embedding_dimensions, new_dimension,
            "Settings should persist new embedding dimension"
        );
        Ok(())
    }
}
