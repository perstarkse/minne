use crate::storage::types::{file_info::deserialize_flexible_id, user::User, StoredObject};
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

    fn get_id(&self) -> &str {
        &self.id
    }
}

impl Analytics {
    pub async fn ensure_initialized(db: &SurrealDbClient) -> Result<Self, AppError> {
        let analytics = db.get_item::<Self>("current").await?;

        if analytics.is_none() {
            let created_analytics = Analytics {
                id: "current".to_string(),
                visitors: 0,
                page_loads: 0,
            };

            let stored: Option<Self> = db.store_item(created_analytics).await?;
            return stored.ok_or(AppError::Validation(
                "Failed to initialize analytics".into(),
            ));
        }

        analytics.ok_or(AppError::Validation(
            "Failed to initialize analytics".into(),
        ))
    }
    pub async fn get_current(db: &SurrealDbClient) -> Result<Self, AppError> {
        let analytics: Option<Self> = db.get_item("current").await?;
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
        // We need to use a direct query for COUNT aggregation
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stored_object;
    use uuid::Uuid;

    stored_object!(TestUser, "user", {
        email: String,
        password: String,
        user_id: String
    });

    #[tokio::test]
    async fn test_analytics_initialization() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Test initialization of analytics
        let analytics = Analytics::ensure_initialized(&db)
            .await
            .expect("Failed to initialize analytics");

        // Verify initial state after initialization
        assert_eq!(analytics.id, "current");
        assert_eq!(analytics.page_loads, 0);
        assert_eq!(analytics.visitors, 0);

        // Test idempotency - ensure calling it again doesn't change anything
        let analytics_again = Analytics::ensure_initialized(&db)
            .await
            .expect("Failed to get analytics after initialization");

        assert_eq!(analytics.id, analytics_again.id);
        assert_eq!(analytics.page_loads, analytics_again.page_loads);
        assert_eq!(analytics.visitors, analytics_again.visitors);
    }

    #[tokio::test]
    async fn test_get_current_analytics() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Initialize analytics
        Analytics::ensure_initialized(&db)
            .await
            .expect("Failed to initialize analytics");

        // Test get_current method
        let analytics = Analytics::get_current(&db)
            .await
            .expect("Failed to get current analytics");

        assert_eq!(analytics.id, "current");
        assert_eq!(analytics.page_loads, 0);
        assert_eq!(analytics.visitors, 0);
    }

    #[tokio::test]
    async fn test_increment_visitors() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Initialize analytics
        Analytics::ensure_initialized(&db)
            .await
            .expect("Failed to initialize analytics");

        // Test increment_visitors method
        let analytics = Analytics::increment_visitors(&db)
            .await
            .expect("Failed to increment visitors");

        assert_eq!(analytics.visitors, 1);
        assert_eq!(analytics.page_loads, 0);

        // Increment again and check
        let analytics = Analytics::increment_visitors(&db)
            .await
            .expect("Failed to increment visitors again");

        assert_eq!(analytics.visitors, 2);
        assert_eq!(analytics.page_loads, 0);
    }

    #[tokio::test]
    async fn test_increment_page_loads() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Initialize analytics
        Analytics::ensure_initialized(&db)
            .await
            .expect("Failed to initialize analytics");

        // Test increment_page_loads method
        let analytics = Analytics::increment_page_loads(&db)
            .await
            .expect("Failed to increment page loads");

        assert_eq!(analytics.visitors, 0);
        assert_eq!(analytics.page_loads, 1);

        // Increment again and check
        let analytics = Analytics::increment_page_loads(&db)
            .await
            .expect("Failed to increment page loads again");

        assert_eq!(analytics.visitors, 0);
        assert_eq!(analytics.page_loads, 2);
    }

    #[tokio::test]
    async fn test_get_users_amount() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Test with no users
        let count = Analytics::get_users_amount(&db)
            .await
            .expect("Failed to get users amount");
        assert_eq!(count, 0);

        // Create a few test users
        for i in 0..3 {
            let user = TestUser {
                id: format!("user{}", i),
                email: format!("user{}@example.com", i),
                password: "password".to_string(),
                user_id: format!("uid{}", i),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            db.store_item(user)
                .await
                .expect("Failed to create test user");
        }

        // Test users amount after adding users
        let count = Analytics::get_users_amount(&db)
            .await
            .expect("Failed to get users amount after adding users");
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_get_current_nonexistent() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Don't initialize analytics and try to get it
        let result = Analytics::get_current(&db).await;

        assert!(result.is_err());
        if let Err(err) = result {
            match err {
                AppError::NotFound(_) => {
                    // Expected error
                }
                _ => panic!("Expected NotFound error, got: {:?}", err),
            }
        }
    }
}
