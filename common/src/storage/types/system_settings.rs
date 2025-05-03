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
    pub query_system_prompt: String,
    pub ingestion_system_prompt: String,
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
    pub async fn ensure_initialized(db: &SurrealDbClient) -> Result<Self, AppError> {
        let settings: Option<Self> = db.get_item("current").await?;

        if settings.is_none() {
            let created_settings = Self::new();
            let stored: Option<Self> = db.store_item(created_settings).await?;
            return stored.ok_or(AppError::Validation("Failed to initialize settings".into()));
        }

        settings.ok_or(AppError::Validation("Failed to initialize settings".into()))
    }

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

    pub fn new() -> Self {
        Self {
            id: "current".to_string(),
            query_system_prompt: crate::storage::types::system_prompts::DEFAULT_QUERY_SYSTEM_PROMPT
                .to_string(),
            ingestion_system_prompt:
                crate::storage::types::system_prompts::DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT
                    .to_string(),
            query_model: "gpt-4o-mini".to_string(),
            processing_model: "gpt-4o-mini".to_string(),
            registrations_enabled: true,
            require_email_verification: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_settings_initialization() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Test initialization of system settings
        let settings = SystemSettings::ensure_initialized(&db)
            .await
            .expect("Failed to initialize system settings");

        // Verify initial state after initialization
        assert_eq!(settings.id, "current");
        assert_eq!(settings.registrations_enabled, true);
        assert_eq!(settings.require_email_verification, false);
        assert_eq!(settings.query_model, "gpt-4o-mini");
        assert_eq!(settings.processing_model, "gpt-4o-mini");
        assert_eq!(
            settings.query_system_prompt,
            crate::storage::types::system_prompts::DEFAULT_QUERY_SYSTEM_PROMPT
        );
        assert_eq!(
            settings.ingestion_system_prompt,
            crate::storage::types::system_prompts::DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT
        );

        // Test idempotency - ensure calling it again doesn't change anything
        let settings_again = SystemSettings::ensure_initialized(&db)
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
        SystemSettings::ensure_initialized(&db)
            .await
            .expect("Failed to initialize system settings");

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
        SystemSettings::ensure_initialized(&db)
            .await
            .expect("Failed to initialize system settings");

        // Create updated settings
        let mut updated_settings = SystemSettings::new();
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
    async fn test_new_method() {
        let settings = SystemSettings::new();

        assert!(settings.id.len() > 0);
        assert_eq!(settings.registrations_enabled, true);
        assert_eq!(settings.require_email_verification, false);
        assert_eq!(settings.query_model, "gpt-4o-mini");
        assert_eq!(settings.processing_model, "gpt-4o-mini");
        assert_eq!(
            settings.query_system_prompt,
            crate::storage::types::system_prompts::DEFAULT_QUERY_SYSTEM_PROMPT
        );
        assert_eq!(
            settings.ingestion_system_prompt,
            crate::storage::types::system_prompts::DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT
        );
    }
}
