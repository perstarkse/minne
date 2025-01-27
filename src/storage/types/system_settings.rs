use crate::storage::types::file_info::deserialize_flexible_id;
use serde::{Deserialize, Serialize};

use crate::{error::AppError, storage::db::SurrealDbClient};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemSettings {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    pub registrations_enabled: bool,
    pub require_email_verification: bool,
}

impl SystemSettings {
    pub async fn ensure_initialized(db: &SurrealDbClient) -> Result<Self, AppError> {
        let settings = db.select(("system_settings", "current")).await?;

        if settings.is_none() {
            let created: Option<SystemSettings> = db
                .create(("system_settings", "current"))
                .content(SystemSettings {
                    id: "current".to_string(),
                    registrations_enabled: true,
                    require_email_verification: false,
                })
                .await?;

            return created.ok_or(AppError::Validation("Failed to initialize settings".into()));
        };

        settings.ok_or(AppError::Validation("Failed to initialize settings".into()))
    }

    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let settings: Option<Self> = db
            .client
            .query("SELECT * FROM type::thing('system_settings', 'current')")
            .await?
            .take(0)?;

        settings.ok_or(AppError::NotFound("System settings not found".into()))
    }

    pub async fn update(db: &SurrealDbClient, changes: Self) -> Result<Self, AppError> {
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
