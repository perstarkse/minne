use crate::storage::types::{file_info::deserialize_flexible_id, user::User, StoredObject};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{error::AppError, storage::db::SurrealDbClient};

#[derive(Debug, Serialize, Deserialize)]
pub struct Analytics {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    pub page_loads: i64,
    pub visitors: i64,
}

impl Analytics {
    pub async fn ensure_initialized(db: &SurrealDbClient) -> Result<Self, AppError> {
        let analytics = db.select(("analytics", "current")).await?;

        if analytics.is_none() {
            let created: Option<Analytics> = db
                .create(("analytics", "current"))
                .content(Analytics {
                    id: "current".to_string(),
                    visitors: 0,
                    page_loads: 0,
                })
                .await?;

            return created.ok_or(AppError::Validation("Failed to initialize settings".into()));
        };

        analytics.ok_or(AppError::Validation("Failed to initialize settings".into()))
    }
    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let analytics: Option<Self> = db
            .client
            .query("SELECT * FROM type::thing('analytics', 'current')")
            .await?
            .take(0)?;

        analytics.ok_or(AppError::NotFound("Analytics not found".into()))
    }

    pub async fn increment_visitors(db: &SurrealDbClient) -> Result<Self, AppError> {
        let updated: Option<Self> = db
            .client
            .query("UPDATE type::thing('analytics', 'current') SET visitors += 1 RETURN AFTER")
            .await?
            .take(0)?;

        updated.ok_or(AppError::Validation("Failed to update analytics".into()))
    }

    pub async fn increment_page_loads(db: &SurrealDbClient) -> Result<Self, AppError> {
        let updated: Option<Self> = db
            .client
            .query("UPDATE type::thing('analytics', 'current') SET page_loads += 1 RETURN AFTER")
            .await?
            .take(0)?;

        updated.ok_or(AppError::Validation("Failed to update analytics".into()))
    }

    pub async fn get_users_amount(db: &SurrealDbClient) -> Result<i64, AppError> {
        #[derive(Debug, Deserialize)]
        struct CountResult {
            count: i64,
        }

        let result: Option<CountResult> = db
            .client
            .query("SELECT count() as count FROM type::table($table) GROUP ALL")
            .bind(("table", User::table_name()))
            .await?
            .take(0)?;

        Ok(result.map(|r| r.count).unwrap_or(0))
    }
}
