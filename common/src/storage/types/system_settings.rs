use chrono::{DateTime, Utc};
use tracing::warn;

use crate::utils::config::EmbeddingBackend;
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
    /// Active embedding backend. Read-only for admin updates; synced from config at startup.
    #[serde(default)]
    pub embedding_backend: Option<EmbeddingBackend>,
    pub query_system_prompt: String,
    pub ingestion_system_prompt: String,
    pub image_processing_model: String,
    pub image_processing_prompt: String,
    pub voice_processing_model: String,
    /// When the maintainer last completed a scheduled `REBUILD INDEX` pass.
    #[serde(default)]
    pub last_index_rebuild_at: Option<DateTime<Utc>>,
    /// Worker id holding the index-rebuild lease, if any.
    #[serde(default)]
    pub index_rebuild_lease_owner: Option<String>,
    /// Lease expiry for in-flight scheduled index rebuilds.
    #[serde(default)]
    pub index_rebuild_lease_expires_at: Option<DateTime<Utc>>,
}

/// Partial update for singleton system settings without cloning unchanged fields.
#[derive(Debug, Default, Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct SystemSettingsPatch {
    pub registrations_enabled: Option<bool>,
    pub require_email_verification: Option<bool>,
    pub query_model: Option<String>,
    pub processing_model: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<u32>,
    pub query_system_prompt: Option<String>,
    pub ingestion_system_prompt: Option<String>,
    pub image_processing_model: Option<String>,
    pub image_processing_prompt: Option<String>,
    pub voice_processing_model: Option<String>,
}

enum UpdateMode {
    User,
    EmbeddingSync,
}

impl StoredObject for SystemSettings {
    fn table_name() -> &'static str {
        "system_settings"
    }

    fn id(&self) -> &str {
        &self.id
    }
}

impl SystemSettingsPatch {
    pub fn apply_to(self, settings: &mut SystemSettings) {
        if let Some(value) = self.registrations_enabled {
            settings.registrations_enabled = value;
        }
        if let Some(value) = self.require_email_verification {
            settings.require_email_verification = value;
        }
        if let Some(value) = self.query_model {
            settings.query_model = value;
        }
        if let Some(value) = self.processing_model {
            settings.processing_model = value;
        }
        if let Some(value) = self.embedding_model {
            settings.embedding_model = value;
        }
        if let Some(value) = self.embedding_dimensions {
            settings.embedding_dimensions = value;
        }
        if let Some(value) = self.query_system_prompt {
            settings.query_system_prompt = value;
        }
        if let Some(value) = self.ingestion_system_prompt {
            settings.ingestion_system_prompt = value;
        }
        if let Some(value) = self.image_processing_model {
            settings.image_processing_model = value;
        }
        if let Some(value) = self.image_processing_prompt {
            settings.image_processing_prompt = value;
        }
        if let Some(value) = self.voice_processing_model {
            settings.voice_processing_model = value;
        }
    }

    pub async fn apply(self, db: &SurrealDbClient) -> Result<SystemSettings, AppError> {
        let mut current = SystemSettings::get_current(db).await?;
        self.apply_to(&mut current);
        SystemSettings::update(db, current).await
    }
}

const INDEX_REBUILD_LEASE_TTL: &str = "6h";

impl SystemSettings {
    pub const RECORD_ID: &'static str = "current";

    #[allow(clippy::result_large_err)]
    fn validate(&self) -> Result<(), AppError> {
        if self.embedding_dimensions == 0 {
            return Err(AppError::Validation(
                "embedding_dimensions must be greater than 0".into(),
            ));
        }

        let model_fields = [
            ("query_model", &self.query_model),
            ("processing_model", &self.processing_model),
            ("embedding_model", &self.embedding_model),
            ("image_processing_model", &self.image_processing_model),
            ("voice_processing_model", &self.voice_processing_model),
        ];
        for (name, value) in model_fields {
            if value.trim().is_empty() {
                return Err(AppError::Validation(format!("{name} must not be empty")));
            }
        }

        let prompt_fields = [
            ("query_system_prompt", &self.query_system_prompt),
            ("ingestion_system_prompt", &self.ingestion_system_prompt),
            ("image_processing_prompt", &self.image_processing_prompt),
        ];
        for (name, value) in prompt_fields {
            if value.trim().is_empty() {
                return Err(AppError::Validation(format!("{name} must not be empty")));
            }
        }

        Ok(())
    }

    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let settings: Option<Self> = db.get_item(Self::RECORD_ID).await?;
        settings.ok_or(AppError::NotFound("system settings not found".into()))
    }

    pub async fn update(db: &SurrealDbClient, changes: Self) -> Result<Self, AppError> {
        Self::update_with_mode(db, changes, UpdateMode::User).await
    }

    async fn update_with_mode(
        db: &SurrealDbClient,
        mut changes: Self,
        mode: UpdateMode,
    ) -> Result<Self, AppError> {
        let current = Self::get_current(db).await?;
        if matches!(mode, UpdateMode::User) {
            changes.embedding_backend = current.embedding_backend;
        }
        changes.id = Self::RECORD_ID.to_string();
        changes.validate()?;

        let updated: Option<Self> = db
            .client
            .query("UPDATE type::thing('system_settings', $id) MERGE $changes RETURN AFTER")
            .bind(("id", Self::RECORD_ID))
            .bind(("changes", changes))
            .await?
            .take(0)?;

        updated.ok_or(AppError::NotFound(
            "system settings record missing after update".into(),
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

        let provider_backend = provider
            .backend_label()
            .parse::<EmbeddingBackend>()
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let provider_dimensions = u32::try_from(provider.dimension()).map_err(|_| {
            AppError::Validation(format!(
                "embedding provider dimension {} exceeds u32::MAX",
                provider.dimension()
            ))
        })?;
        let provider_model = provider.model_code();

        if settings.embedding_backend != Some(provider_backend) {
            settings.embedding_backend = Some(provider_backend);
            needs_update = true;
        }

        if settings.embedding_dimensions != provider_dimensions {
            tracing::info!(
                old_dimensions = settings.embedding_dimensions,
                new_dimensions = provider_dimensions,
                "Embedding dimensions changed, updating SystemSettings"
            );
            settings.embedding_dimensions = provider_dimensions;
            needs_update = true;
        }

        if let Some(model) = provider_model
            && settings.embedding_model != model
        {
            tracing::info!(
                old_model = %settings.embedding_model,
                new_model = %model,
                "Embedding model changed, updating SystemSettings"
            );
            settings.embedding_model = model;
            needs_update = true;
        }

        if needs_update {
            settings = Self::update_with_mode(db, settings, UpdateMode::EmbeddingSync).await?;
        }

        Ok((settings, needs_update))
    }

    /// Seeds the first rebuild checkpoint so the initial scheduled rebuild waits one interval.
    pub async fn seed_index_rebuild_checkpoint(db: &SurrealDbClient) -> Result<bool, AppError> {
        let mut response = db
            .client
            .query(
                "UPDATE type::thing('system_settings', $id) SET last_index_rebuild_at = time::now()
                 WHERE last_index_rebuild_at IS NONE
                 RETURN AFTER;",
            )
            .bind(("id", Self::RECORD_ID))
            .await
            .map_err(AppError::from)?;

        let updated: Option<Self> = response.take(0).map_err(AppError::from)?;
        Ok(updated.is_some())
    }

    /// Claims the singleton index-rebuild lease when it is free or expired.
    pub async fn try_acquire_index_rebuild_lease(
        db: &SurrealDbClient,
        owner: &str,
    ) -> Result<bool, AppError> {
        let mut response = db
            .client
            .query(format!(
                "UPDATE type::thing('system_settings', $id) SET
                    index_rebuild_lease_owner = $owner,
                    index_rebuild_lease_expires_at = time::now() + {INDEX_REBUILD_LEASE_TTL}
                 WHERE index_rebuild_lease_expires_at IS NONE
                    OR index_rebuild_lease_expires_at < time::now()
                 RETURN AFTER;"
            ))
            .bind(("id", Self::RECORD_ID))
            .bind(("owner", owner.to_string()))
            .await
            .map_err(AppError::from)?;

        let updated: Option<Self> = response.take(0).map_err(AppError::from)?;
        Ok(updated.is_some())
    }

    /// Releases the index-rebuild lease when held by `owner`.
    pub async fn release_index_rebuild_lease(db: &SurrealDbClient, owner: &str) {
        let released = db
            .client
            .query(
                "UPDATE type::thing('system_settings', $id) SET
                    index_rebuild_lease_owner = NONE,
                    index_rebuild_lease_expires_at = NONE
                 WHERE index_rebuild_lease_owner = $owner;",
            )
            .bind(("id", Self::RECORD_ID))
            .bind(("owner", owner.to_string()))
            .await
            .and_then(surrealdb::Response::check);

        if let Err(err) = released {
            warn!(error = %err, "failed to release index rebuild lease");
        }
    }

    /// Records a completed scheduled index rebuild and clears the lease.
    pub async fn record_index_rebuild_completed(
        db: &SurrealDbClient,
        owner: &str,
    ) -> Result<(), AppError> {
        let response = db
            .client
            .query(
                "UPDATE type::thing('system_settings', $id) SET
                    last_index_rebuild_at = time::now(),
                    index_rebuild_lease_owner = NONE,
                    index_rebuild_lease_expires_at = NONE
                 WHERE index_rebuild_lease_owner = $owner;",
            )
            .bind(("id", Self::RECORD_ID))
            .bind(("owner", owner.to_string()))
            .await
            .map_err(AppError::from)?;
        response.check().map_err(AppError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use crate::storage::indexes::ensure_runtime;
    use crate::storage::types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk};
    use anyhow::{self, Context};

    use super::*;
    use crate::test_utils::setup_test_db;
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
        let db = setup_test_db().await?;
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
        assert!(!settings.ingestion_system_prompt.contains("entity\"\n6."));
        assert!(settings.ingestion_system_prompt.contains("related entity."));

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
        let db = setup_test_db().await?;

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
        let db = setup_test_db().await?;

        // Create updated settings
        let mut updated_settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "get_current".to_string())?;
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
        let db = SurrealDbClient::memory("test_ns", &Uuid::new_v4().to_string()).await?;
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
    async fn test_update_rejects_zero_embedding_dimensions() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let mut invalid_settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get system settings".to_string())?;
        invalid_settings.embedding_dimensions = 0;

        let result = SystemSettings::update(&db, invalid_settings).await;
        assert!(matches!(result, Err(AppError::Validation(_))));
        Ok(())
    }

    #[tokio::test]
    async fn test_patch_updates_without_cloning_full_settings() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let updated = SystemSettingsPatch {
            registrations_enabled: Some(false),
            ..Default::default()
        }
        .apply(&db)
        .await
        .with_context(|| "Failed to patch settings".to_string())?;

        assert!(!updated.registrations_enabled);
        Ok(())
    }

    #[tokio::test]
    async fn test_patch_leaves_unmentioned_fields_unchanged() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let original = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get system settings".to_string())?;
        let sentinel = "custom-query-prompt-sentinel".to_string();

        let patched = SystemSettingsPatch {
            query_system_prompt: Some(sentinel.clone()),
            ..Default::default()
        }
        .apply(&db)
        .await
        .with_context(|| "Failed to patch query prompt".to_string())?;

        assert_eq!(patched.query_system_prompt, sentinel);
        assert_eq!(
            patched.ingestion_system_prompt,
            original.ingestion_system_prompt
        );
        assert_eq!(patched.query_model, original.query_model);
        assert_eq!(
            patched.registrations_enabled,
            original.registrations_enabled
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_update_rejects_empty_model_name() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let mut invalid_settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get system settings".to_string())?;
        invalid_settings.query_model = "   ".to_string();

        let result = SystemSettings::update(&db, invalid_settings).await;
        assert!(matches!(result, Err(AppError::Validation(_))));
        Ok(())
    }

    #[tokio::test]
    async fn test_update_normalizes_record_id() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let mut settings = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get system settings".to_string())?;
        settings.id = "wrong-id".to_string();

        let updated = SystemSettings::update(&db, settings)
            .await
            .with_context(|| "Failed to update settings".to_string())?;
        assert_eq!(updated.id, SystemSettings::RECORD_ID);
        Ok(())
    }

    #[tokio::test]
    async fn test_update_preserves_embedding_backend() -> anyhow::Result<()> {
        use crate::utils::embedding::EmbeddingProvider;

        let db = setup_test_db().await?;

        let provider = EmbeddingProvider::new_hashed(384)
            .with_context(|| "Failed to create hashed embedding provider".to_string())?;
        SystemSettings::sync_from_embedding_provider(&db, &provider)
            .await
            .with_context(|| "Failed to sync embedding provider".to_string())?;

        let synced = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to get synced settings".to_string())?;
        assert_eq!(synced.embedding_backend, Some(EmbeddingBackend::Hashed));

        let mut tampered = synced;
        tampered.embedding_backend = Some(EmbeddingBackend::OpenAI);
        let updated = SystemSettings::update(&db, tampered)
            .await
            .with_context(|| "Failed to update settings".to_string())?;

        assert_eq!(updated.embedding_backend, Some(EmbeddingBackend::Hashed));
        Ok(())
    }

    #[tokio::test]
    async fn test_sync_from_embedding_provider_updates_mismatched_settings() -> anyhow::Result<()> {
        use crate::utils::embedding::EmbeddingProvider;

        let db = setup_test_db().await?;

        let provider = EmbeddingProvider::new_hashed(384)
            .with_context(|| "Failed to create hashed embedding provider".to_string())?;
        let (settings, changed) = SystemSettings::sync_from_embedding_provider(&db, &provider)
            .await
            .with_context(|| "Failed to sync embedding provider".to_string())?;

        assert!(changed);
        assert_eq!(settings.embedding_backend, Some(EmbeddingBackend::Hashed));
        assert_eq!(settings.embedding_dimensions, 384);

        let persisted = SystemSettings::get_current(&db)
            .await
            .with_context(|| "Failed to reload synced settings".to_string())?;
        assert_eq!(persisted.embedding_backend, Some(EmbeddingBackend::Hashed));
        assert_eq!(persisted.embedding_dimensions, 384);
        Ok(())
    }

    #[tokio::test]
    async fn test_sync_from_embedding_provider_is_noop_when_already_synced() -> anyhow::Result<()> {
        use crate::utils::embedding::EmbeddingProvider;

        let db = setup_test_db().await?;

        let provider = EmbeddingProvider::new_hashed(384)
            .with_context(|| "Failed to create hashed embedding provider".to_string())?;
        SystemSettings::sync_from_embedding_provider(&db, &provider)
            .await
            .with_context(|| "Failed to initial sync".to_string())?;

        let (_, changed) = SystemSettings::sync_from_embedding_provider(&db, &provider)
            .await
            .with_context(|| "Failed to repeat sync".to_string())?;
        assert!(!changed);
        Ok(())
    }

    #[tokio::test]
    async fn test_sync_rejects_provider_dimension_above_u32_max() -> anyhow::Result<()> {
        use crate::utils::embedding::EmbeddingProvider;

        let db = setup_test_db().await?;

        let provider = EmbeddingProvider::new_hashed((u32::MAX as usize) + 1)
            .with_context(|| "Failed to create oversized hashed provider".to_string())?;
        let result = SystemSettings::sync_from_embedding_provider(&db, &provider).await;
        assert!(matches!(result, Err(AppError::Validation(_))));
        Ok(())
    }

    #[tokio::test]
    async fn test_migration_after_changing_embedding_length() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let initial_chunk = TextChunk::new(
            "source1".into(),
            "This chunk has the original dimension".into(),
            "user1".into(),
        );

        TextChunk::store_with_embedding(initial_chunk.clone(), vec![0.1; 1536], 1536, &db)
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
    async fn test_should_change_embedding_length_on_indexes_when_switching_length()
    -> anyhow::Result<()> {
        use crate::utils::embedding::EmbeddingProvider;

        let db = setup_test_db().await?;

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

        let provider = EmbeddingProvider::new_hashed(new_dimension as usize)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        TextChunk::update_all_embeddings(&db, &provider)
            .await
            .with_context(|| "TextChunk re-embedding should succeed on fresh DB".to_string())?;
        KnowledgeEntity::update_all_embeddings(&db, &provider)
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

    #[tokio::test]
    async fn index_rebuild_lease_is_exclusive_on_system_settings() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        assert!(
            SystemSettings::try_acquire_index_rebuild_lease(&db, "worker-a").await?,
            "first lease claim should succeed"
        );
        assert!(
            !SystemSettings::try_acquire_index_rebuild_lease(&db, "worker-b").await?,
            "second lease claim should fail while lease is held"
        );

        SystemSettings::release_index_rebuild_lease(&db, "worker-a").await;

        SystemSettings::try_acquire_index_rebuild_lease(&db, "worker-b").await?;
        SystemSettings::record_index_rebuild_completed(&db, "worker-b").await?;

        let settings = SystemSettings::get_current(&db).await?;
        assert!(settings.last_index_rebuild_at.is_some());
        assert!(settings.index_rebuild_lease_owner.is_none());
        Ok(())
    }
}
