use crate::storage::types::{user::User, StoredObject};
use crate::utils::serde_helpers::deserialize_flexible_id;
use serde::{Deserialize, Serialize};

use crate::{error::AppError, storage::db::SurrealDbClient};

#[derive(Debug, Serialize, Deserialize)]
pub struct Analytics {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    pub page_loads: i64,
    pub visitors: i64,
}

impl StoredObject for Analytics {
    fn table_name() -> &'static str {
        "analytics"
    }

    fn id(&self) -> &str {
        &self.id
    }
}

impl Analytics {
    const RECORD_ID: &'static str = "current";

    /// Ensures the singleton analytics record exists (idempotent).
    ///
    /// Production databases are also seeded by `20250503_215025_initial_setup.surql`;
    /// this uses an atomic `UPSERT` for tests and recovery.
    pub async fn ensure_initialized(db: &SurrealDbClient) -> Result<Self, AppError> {
        let analytics: Option<Self> = db
            .client
            .query(
                "UPSERT type::thing('analytics', $id) SET visitors = visitors ?? 0, page_loads = page_loads ?? 0 RETURN AFTER",
            )
            .bind(("id", Self::RECORD_ID))
            .await?
            .take(0)?;

        analytics.ok_or(AppError::Validation(
            "failed to initialize analytics".into(),
        ))
    }
    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let analytics: Option<Self> = db.get_item("current").await?;
        analytics.ok_or(AppError::NotFound("analytics not found".into()))
    }

    pub async fn increment_visitors(db: &SurrealDbClient) -> Result<Self, AppError> {
        let updated: Option<Self> = db
            .client
            .query(
                "UPSERT type::thing('analytics', $id) SET visitors = (visitors ?? 0) + 1, page_loads = page_loads ?? 0 RETURN AFTER",
            )
            .bind(("id", Self::RECORD_ID))
            .await?
            .take(0)?;

        updated.ok_or(AppError::Validation("failed to update analytics".into()))
    }

    pub async fn increment_page_loads(db: &SurrealDbClient) -> Result<Self, AppError> {
        Self::record_page_view(db, false).await
    }

    /// Records a page view, optionally counting the visitor as new.
    pub async fn record_page_view(
        db: &SurrealDbClient,
        is_new_visitor: bool,
    ) -> Result<Self, AppError> {
        let visitor_delta = i64::from(is_new_visitor);
        let updated: Option<Self> = db
            .client
            .query(
                "UPSERT type::thing('analytics', $id) SET page_loads = (page_loads ?? 0) + 1, visitors = (visitors ?? 0) + $visitor_delta RETURN AFTER",
            )
            .bind(("id", Self::RECORD_ID))
            .bind(("visitor_delta", visitor_delta))
            .await?
            .take(0)?;

        updated.ok_or(AppError::Validation("failed to update analytics".into()))
    }

    pub async fn get_users_amount(db: &SurrealDbClient) -> Result<i64, AppError> {
        // We need to use a direct query for COUNT aggregation
        #[derive(Debug, Deserialize)]
        struct CountResult {
            /// Total user count.
            count: i64,
        }

        let result: Option<CountResult> = db
            .client
            .query("SELECT count() as count FROM type::table($table) GROUP ALL")
            .bind(("table", User::table_name()))
            .await?
            .take(0)?;

        Ok(result.map_or(0, |r| r.count))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use super::*;
    use crate::stored_object;
    use anyhow::{self};
    use uuid::Uuid;

    stored_object!(TestUser, "user", {
        email: String,
        password: String,
        user_id: String
    });

    #[tokio::test]
    async fn test_analytics_initialization() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        // Test initialization of analytics
        let analytics = Analytics::ensure_initialized(&db).await?;

        // Verify initial state after initialization
        assert_eq!(analytics.id, "current");
        assert_eq!(analytics.page_loads, 0);
        assert_eq!(analytics.visitors, 0);

        // Test idempotency - ensure calling it again doesn't change anything
        let analytics_again = Analytics::ensure_initialized(&db).await?;

        assert_eq!(analytics.id, analytics_again.id);
        assert_eq!(analytics.page_loads, analytics_again.page_loads);
        assert_eq!(analytics.visitors, analytics_again.visitors);

        Ok(())
    }

    #[tokio::test]
    async fn test_get_current_analytics() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        // Initialize analytics
        Analytics::ensure_initialized(&db).await?;

        // Test get_current method
        let analytics = Analytics::get_current(&db).await?;

        assert_eq!(analytics.id, "current");
        assert_eq!(analytics.page_loads, 0);
        assert_eq!(analytics.visitors, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_increment_visitors() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        // Initialize analytics
        Analytics::ensure_initialized(&db).await?;

        // Test increment_visitors method
        let analytics = Analytics::increment_visitors(&db).await?;

        assert_eq!(analytics.visitors, 1);
        assert_eq!(analytics.page_loads, 0);

        // Increment again and check
        let analytics = Analytics::increment_visitors(&db).await?;

        assert_eq!(analytics.visitors, 2);
        assert_eq!(analytics.page_loads, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_increment_page_loads() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        // Initialize analytics
        Analytics::ensure_initialized(&db).await?;

        // Test increment_page_loads method
        let analytics = Analytics::increment_page_loads(&db).await?;

        assert_eq!(analytics.visitors, 0);
        assert_eq!(analytics.page_loads, 1);

        // Increment again and check
        let analytics = Analytics::increment_page_loads(&db).await?;

        assert_eq!(analytics.visitors, 0);
        assert_eq!(analytics.page_loads, 2);

        Ok(())
    }

    #[tokio::test]
    async fn test_get_users_amount() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        // Test with no users
        let count = Analytics::get_users_amount(&db).await?;
        assert_eq!(count, 0);

        // Create a few test users
        for i in 0..3 {
            let user = TestUser {
                id: format!("user{i}"),
                email: format!("user{i}@example.com"),
                password: "password".to_string(),
                user_id: format!("uid{i}"),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            db.store_item(user).await?;
        }

        // Test users amount after adding users
        let count = Analytics::get_users_amount(&db).await?;
        assert_eq!(count, 3);

        Ok(())
    }

    #[tokio::test]
    async fn test_increment_visitors_without_prior_init() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        let analytics = Analytics::increment_visitors(&db).await?;
        assert_eq!(analytics.visitors, 1);
        assert_eq!(analytics.page_loads, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_increment_page_loads_without_prior_init() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        let analytics = Analytics::increment_page_loads(&db).await?;
        assert_eq!(analytics.page_loads, 1);
        assert_eq!(analytics.visitors, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_visitor_and_page_load_increments_are_independent() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        let after_visitors = Analytics::increment_visitors(&db).await?;
        assert_eq!(after_visitors.visitors, 1);
        assert_eq!(after_visitors.page_loads, 0);

        let after_page_load = Analytics::increment_page_loads(&db).await?;
        assert_eq!(after_page_load.visitors, 1);
        assert_eq!(after_page_load.page_loads, 1);

        let after_second_visitor = Analytics::increment_visitors(&db).await?;
        assert_eq!(after_second_visitor.visitors, 2);
        assert_eq!(after_second_visitor.page_loads, 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_record_page_view() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        let first_view = Analytics::record_page_view(&db, true).await?;
        assert_eq!(first_view.visitors, 1);
        assert_eq!(first_view.page_loads, 1);

        let returning_view = Analytics::record_page_view(&db, false).await?;
        assert_eq!(returning_view.visitors, 1);
        assert_eq!(returning_view.page_loads, 2);

        Ok(())
    }

    #[tokio::test]
    async fn test_get_current_nonexistent() -> anyhow::Result<()> {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        // Don't initialize analytics and try to get it
        let result = Analytics::get_current(&db).await;

        assert!(result.is_err());
        match result {
            Ok(_) => anyhow::bail!("Expected NotFound error, got success"),
            Err(AppError::NotFound(_)) => {}
            Err(err) => anyhow::bail!("Expected NotFound error, got: {err:?}"),
        }

        Ok(())
    }
}
