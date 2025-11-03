use chrono::Utc as ChronoUtc;
use surrealdb::opt::PatchOp;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

stored_object!(Scratchpad, "scratchpad", {
    user_id: String,
    title: String,
    content: String,
    #[serde(serialize_with = "serialize_datetime", deserialize_with="deserialize_datetime")]
    last_saved_at: DateTime<Utc>,
    is_dirty: bool,
    #[serde(default)]
    is_archived: bool,
    #[serde(
        serialize_with = "serialize_option_datetime",
        deserialize_with = "deserialize_option_datetime",
        default
    )]
    archived_at: Option<DateTime<Utc>>,
    #[serde(
        serialize_with = "serialize_option_datetime",
        deserialize_with = "deserialize_option_datetime",
        default
    )]
    ingested_at: Option<DateTime<Utc>>
});

impl Scratchpad {
    pub fn new(user_id: String, title: String) -> Self {
        let now = ChronoUtc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            user_id,
            title,
            content: String::new(),
            last_saved_at: now,
            is_dirty: false,
            is_archived: false,
            archived_at: None,
            ingested_at: None,
        }
    }

    pub async fn get_by_user(user_id: &str, db: &SurrealDbClient) -> Result<Vec<Self>, AppError> {
        let scratchpads: Vec<Scratchpad> = db.client
            .query("SELECT * FROM type::table($table_name) WHERE user_id = $user_id AND (is_archived = false OR is_archived IS NONE) ORDER BY updated_at DESC")
            .bind(("table_name", Self::table_name()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;

        Ok(scratchpads)
    }

    pub async fn get_archived_by_user(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<Self>, AppError> {
        let scratchpads: Vec<Scratchpad> = db.client
            .query("SELECT * FROM type::table($table_name) WHERE user_id = $user_id AND is_archived = true ORDER BY archived_at DESC, updated_at DESC")
            .bind(("table_name", Self::table_name()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;

        Ok(scratchpads)
    }

    pub async fn get_by_id(
        id: &str,
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Self, AppError> {
        let scratchpad: Option<Scratchpad> = db.get_item(id).await?;

        let scratchpad =
            scratchpad.ok_or_else(|| AppError::NotFound("Scratchpad not found".to_string()))?;

        if scratchpad.user_id != user_id {
            return Err(AppError::Auth(
                "You don't have access to this scratchpad".to_string(),
            ));
        }

        Ok(scratchpad)
    }

    pub async fn update_content(
        id: &str,
        user_id: &str,
        new_content: &str,
        db: &SurrealDbClient,
    ) -> Result<Self, AppError> {
        // First verify ownership
        let scratchpad = Self::get_by_id(id, user_id, db).await?;

        if scratchpad.is_archived {
            return Ok(scratchpad);
        }

        let now = ChronoUtc::now();
        let _updated: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/content", new_content.to_string()))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::Datetime::from(now),
            ))
            .patch(PatchOp::replace(
                "/last_saved_at",
                surrealdb::Datetime::from(now),
            ))
            .patch(PatchOp::replace("/is_dirty", false))
            .await?;

        // Return the updated scratchpad
        Self::get_by_id(id, user_id, db).await
    }

    pub async fn update_title(
        id: &str,
        user_id: &str,
        new_title: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        // First verify ownership
        let _scratchpad = Self::get_by_id(id, user_id, db).await?;

        let _updated: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/title", new_title.to_string()))
            .patch(PatchOp::replace(
                "/updated_at",
                surrealdb::Datetime::from(ChronoUtc::now()),
            ))
            .await?;

        Ok(())
    }

    pub async fn delete(id: &str, user_id: &str, db: &SurrealDbClient) -> Result<(), AppError> {
        // First verify ownership
        let _scratchpad = Self::get_by_id(id, user_id, db).await?;

        let _: Option<Self> = db.client.delete((Self::table_name(), id)).await?;

        Ok(())
    }

    pub async fn archive(
        id: &str,
        user_id: &str,
        db: &SurrealDbClient,
        mark_ingested: bool,
    ) -> Result<Self, AppError> {
        // Verify ownership
        let scratchpad = Self::get_by_id(id, user_id, db).await?;

        if scratchpad.is_archived {
            if mark_ingested && scratchpad.ingested_at.is_none() {
                // Ensure ingested_at is set if required
                let surreal_now = surrealdb::Datetime::from(ChronoUtc::now());
                let _updated: Option<Self> = db
                    .update((Self::table_name(), id))
                    .patch(PatchOp::replace("/ingested_at", surreal_now))
                    .await?;
                return Self::get_by_id(id, user_id, db).await;
            }
            return Ok(scratchpad);
        }

        let now = ChronoUtc::now();
        let surreal_now = surrealdb::Datetime::from(now);
        let mut update = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/is_archived", true))
            .patch(PatchOp::replace("/archived_at", surreal_now.clone()))
            .patch(PatchOp::replace("/updated_at", surreal_now.clone()));

        update = if mark_ingested {
            update.patch(PatchOp::replace("/ingested_at", surreal_now))
        } else {
            update.patch(PatchOp::remove("/ingested_at"))
        };

        let _updated: Option<Self> = update.await?;

        Self::get_by_id(id, user_id, db).await
    }

    pub async fn restore(id: &str, user_id: &str, db: &SurrealDbClient) -> Result<Self, AppError> {
        // Verify ownership
        let scratchpad = Self::get_by_id(id, user_id, db).await?;

        if !scratchpad.is_archived {
            return Ok(scratchpad);
        }

        let now = ChronoUtc::now();
        let surreal_now = surrealdb::Datetime::from(now);
        let _updated: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/is_archived", false))
            .patch(PatchOp::remove("/archived_at"))
            .patch(PatchOp::remove("/ingested_at"))
            .patch(PatchOp::replace("/updated_at", surreal_now))
            .await?;

        Self::get_by_id(id, user_id, db).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_scratchpad() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        // Create a new scratchpad
        let user_id = "test_user";
        let title = "Test Scratchpad";
        let scratchpad = Scratchpad::new(user_id.to_string(), title.to_string());

        // Verify scratchpad properties
        assert_eq!(scratchpad.user_id, user_id);
        assert_eq!(scratchpad.title, title);
        assert_eq!(scratchpad.content, "");
        assert!(!scratchpad.is_dirty);
        assert!(!scratchpad.is_archived);
        assert!(scratchpad.archived_at.is_none());
        assert!(scratchpad.ingested_at.is_none());
        assert!(!scratchpad.id.is_empty());

        // Store the scratchpad
        let result = db.store_item(scratchpad.clone()).await;
        assert!(result.is_ok());

        // Verify it can be retrieved
        let retrieved: Option<Scratchpad> = db
            .get_item(&scratchpad.id)
            .await
            .expect("Failed to retrieve scratchpad");
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, scratchpad.id);
        assert_eq!(retrieved.user_id, user_id);
        assert_eq!(retrieved.title, title);
        assert!(!retrieved.is_archived);
        assert!(retrieved.archived_at.is_none());
        assert!(retrieved.ingested_at.is_none());
    }

    #[tokio::test]
    async fn test_get_by_user() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let user_id = "test_user";

        // Create multiple scratchpads
        let scratchpad1 = Scratchpad::new(user_id.to_string(), "First".to_string());
        let scratchpad2 = Scratchpad::new(user_id.to_string(), "Second".to_string());
        let scratchpad3 = Scratchpad::new("other_user".to_string(), "Other".to_string());

        // Store them
        let scratchpad1_id = scratchpad1.id.clone();
        let scratchpad2_id = scratchpad2.id.clone();
        db.store_item(scratchpad1).await.unwrap();
        db.store_item(scratchpad2).await.unwrap();
        db.store_item(scratchpad3).await.unwrap();

        // Archive one of the user's scratchpads
        Scratchpad::archive(&scratchpad2_id, user_id, &db, false)
            .await
            .unwrap();

        // Get scratchpads for user_id
        let user_scratchpads = Scratchpad::get_by_user(user_id, &db).await.unwrap();
        assert_eq!(user_scratchpads.len(), 1);
        assert_eq!(user_scratchpads[0].id, scratchpad1_id);

        // Verify they belong to the user
        for scratchpad in &user_scratchpads {
            assert_eq!(scratchpad.user_id, user_id);
        }

        let archived = Scratchpad::get_archived_by_user(user_id, &db)
            .await
            .unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, scratchpad2_id);
        assert!(archived[0].is_archived);
        assert!(archived[0].ingested_at.is_none());
    }

    #[tokio::test]
    async fn test_archive_and_restore() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let user_id = "test_user";
        let scratchpad = Scratchpad::new(user_id.to_string(), "Test".to_string());
        let scratchpad_id = scratchpad.id.clone();
        db.store_item(scratchpad).await.unwrap();

        let archived = Scratchpad::archive(&scratchpad_id, user_id, &db, true)
            .await
            .expect("Failed to archive");
        assert!(archived.is_archived);
        assert!(archived.archived_at.is_some());
        assert!(archived.ingested_at.is_some());

        let restored = Scratchpad::restore(&scratchpad_id, user_id, &db)
            .await
            .expect("Failed to restore");
        assert!(!restored.is_archived);
        assert!(restored.archived_at.is_none());
        assert!(restored.ingested_at.is_none());
    }

    #[tokio::test]
    async fn test_update_content() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let user_id = "test_user";
        let scratchpad = Scratchpad::new(user_id.to_string(), "Test".to_string());
        let scratchpad_id = scratchpad.id.clone();

        db.store_item(scratchpad).await.unwrap();

        let new_content = "Updated content";
        let updated = Scratchpad::update_content(&scratchpad_id, user_id, new_content, &db)
            .await
            .unwrap();

        assert_eq!(updated.content, new_content);
        assert!(!updated.is_dirty);
    }

    #[tokio::test]
    async fn test_update_content_unauthorized() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let owner_id = "owner";
        let other_user = "other_user";
        let scratchpad = Scratchpad::new(owner_id.to_string(), "Test".to_string());
        let scratchpad_id = scratchpad.id.clone();

        db.store_item(scratchpad).await.unwrap();

        let result = Scratchpad::update_content(&scratchpad_id, other_user, "Hacked", &db).await;
        assert!(result.is_err());
        match result {
            Err(AppError::Auth(_)) => {}
            _ => panic!("Expected Auth error"),
        }
    }

    #[tokio::test]
    async fn test_delete_scratchpad() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let user_id = "test_user";
        let scratchpad = Scratchpad::new(user_id.to_string(), "Test".to_string());
        let scratchpad_id = scratchpad.id.clone();

        db.store_item(scratchpad).await.unwrap();

        // Delete should succeed
        let result = Scratchpad::delete(&scratchpad_id, user_id, &db).await;
        assert!(result.is_ok());

        // Verify it's gone
        let retrieved: Option<Scratchpad> = db.get_item(&scratchpad_id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_delete_unauthorized() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let owner_id = "owner";
        let other_user = "other_user";
        let scratchpad = Scratchpad::new(owner_id.to_string(), "Test".to_string());
        let scratchpad_id = scratchpad.id.clone();

        db.store_item(scratchpad).await.unwrap();

        let result = Scratchpad::delete(&scratchpad_id, other_user, &db).await;
        assert!(result.is_err());
        match result {
            Err(AppError::Auth(_)) => {}
            _ => panic!("Expected Auth error"),
        }

        // Verify it still exists
        let retrieved: Option<Scratchpad> = db.get_item(&scratchpad_id).await.unwrap();
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_timezone_aware_scratchpad_conversion() {
        let db = SurrealDbClient::memory("test_ns", &Uuid::new_v4().to_string())
            .await
            .expect("Failed to create test database");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let user_id = "test_user_123";
        let scratchpad =
            Scratchpad::new(user_id.to_string(), "Test Timezone Scratchpad".to_string());
        let scratchpad_id = scratchpad.id.clone();

        db.store_item(scratchpad).await.unwrap();

        let retrieved = Scratchpad::get_by_id(&scratchpad_id, user_id, &db)
            .await
            .unwrap();

        // Test that datetime fields are preserved and can be used for timezone formatting
        assert!(retrieved.created_at.timestamp() > 0);
        assert!(retrieved.updated_at.timestamp() > 0);
        assert!(retrieved.last_saved_at.timestamp() > 0);

        // Test that optional datetime fields work correctly
        assert!(retrieved.archived_at.is_none());
        assert!(retrieved.ingested_at.is_none());

        // Archive the scratchpad to test optional datetime handling
        let archived = Scratchpad::archive(&scratchpad_id, user_id, &db, false)
            .await
            .unwrap();

        assert!(archived.archived_at.is_some());
        assert!(archived.archived_at.unwrap().timestamp() > 0);
        assert!(archived.ingested_at.is_none());
    }
}
