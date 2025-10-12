use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};
use async_trait::async_trait;
use axum_session_auth::Authentication;
use chrono_tz::Tz;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

use super::text_chunk::TextChunk;
use super::{
    conversation::Conversation,
    ingestion_task::{IngestionTask, TaskState},
    knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
    knowledge_relationship::KnowledgeRelationship,
    system_settings::SystemSettings,
    text_content::TextContent,
};
use chrono::Duration;
use futures::try_join;

#[derive(Deserialize)]
pub struct CategoryResponse {
    category: String,
}

stored_object!(User, "user", {
    email: String,
    password: String,
    anonymous: bool,
    api_key: Option<String>,
    admin: bool,
    #[serde(default)]
    timezone: String
});

#[async_trait]
impl Authentication<User, String, Surreal<Any>> for User {
    async fn load_user(userid: String, db: Option<&Surreal<Any>>) -> Result<User, anyhow::Error> {
        let db = db.unwrap();
        Ok(db
            .select((Self::table_name(), userid.as_str()))
            .await?
            .unwrap())
    }

    fn is_authenticated(&self) -> bool {
        !self.anonymous
    }

    fn is_active(&self) -> bool {
        !self.anonymous
    }

    fn is_anonymous(&self) -> bool {
        self.anonymous
    }
}

fn validate_timezone(input: &str) -> String {
    match input.parse::<Tz>() {
        Ok(_) => input.to_owned(),
        Err(_) => {
            tracing::warn!("Invalid timezone '{}' received, defaulting to UTC", input);
            "UTC".to_owned()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DashboardStats {
    pub total_documents: i64,
    pub new_documents_week: i64,
    pub total_entities: i64,
    pub new_entities_week: i64,
    pub total_conversations: i64,
    pub new_conversations_week: i64,
    pub total_text_chunks: i64,
    pub new_text_chunks_week: i64,
}

#[derive(Deserialize)]
struct CountResult {
    count: i64,
}

impl User {
    async fn count_total<T: crate::storage::types::StoredObject>(
        db: &SurrealDbClient,
        user_id: &str,
    ) -> Result<i64, AppError> {
        let result: Option<CountResult> = db
            .client
            .query("SELECT count() as count FROM type::table($table) WHERE user_id = $user_id GROUP ALL")
            .bind(("table", T::table_name()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    async fn count_since<T: crate::storage::types::StoredObject>(
        db: &SurrealDbClient,
        user_id: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64, AppError> {
        let result: Option<CountResult> = db
            .client
            .query(
                "SELECT count() as count FROM type::table($table) WHERE user_id = $user_id AND created_at >= $since GROUP ALL",
            )
            .bind(("table", T::table_name()))
            .bind(("user_id", user_id.to_string()))
            .bind(("since", surrealdb::Datetime::from(since)))
            .await?
            .take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    pub async fn get_dashboard_stats(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<DashboardStats, AppError> {
        let since = chrono::Utc::now() - Duration::days(7);

        let (
            total_documents,
            new_documents_week,
            total_entities,
            new_entities_week,
            total_conversations,
            new_conversations_week,
            total_text_chunks,
            new_text_chunks_week,
        ) = try_join!(
            Self::count_total::<TextContent>(db, user_id),
            Self::count_since::<TextContent>(db, user_id, since),
            Self::count_total::<KnowledgeEntity>(db, user_id),
            Self::count_since::<KnowledgeEntity>(db, user_id, since),
            Self::count_total::<Conversation>(db, user_id),
            Self::count_since::<Conversation>(db, user_id, since),
            Self::count_total::<TextChunk>(db, user_id),
            Self::count_since::<TextChunk>(db, user_id, since)
        )?;

        Ok(DashboardStats {
            total_documents,
            new_documents_week,
            total_entities,
            new_entities_week,
            total_conversations,
            new_conversations_week,
            total_text_chunks,
            new_text_chunks_week,
        })
    }
    pub async fn create_new(
        email: String,
        password: String,
        db: &SurrealDbClient,
        timezone: String,
    ) -> Result<Self, AppError> {
        // verify that the application allows new creations
        let systemsettings = SystemSettings::get_current(db).await?;
        if !systemsettings.registrations_enabled {
            return Err(AppError::Auth("Registration is not allowed".into()));
        }

        let validated_tz = validate_timezone(&timezone);
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();

        let user: Option<User> = db
            .client
            .query(
                "LET $count = (SELECT count() FROM type::table($table))[0].count;
             CREATE type::thing('user', $id) SET
                email = $email,
                password = crypto::argon2::generate($password),
                admin = $count < 1,
                anonymous = false,
                created_at = $created_at,
                updated_at = $updated_at,
                timezone = $timezone",
            )
            .bind(("table", "user"))
            .bind(("id", id))
            .bind(("email", email))
            .bind(("password", password))
            .bind(("created_at", surrealdb::Datetime::from(now)))
            .bind(("updated_at", surrealdb::Datetime::from(now)))
            .bind(("timezone", validated_tz))
            .await?
            .take(1)?;

        user.ok_or(AppError::Auth("User failed to create".into()))
    }

    pub async fn patch_password(
        email: &str,
        password: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        db.client
            .query(
                "UPDATE user
            SET password = crypto::argon2::generate($password)
            WHERE email = $email",
            )
            .bind(("email", email.to_owned()))
            .bind(("password", password.to_owned()))
            .await?;

        Ok(())
    }

    pub async fn authenticate(
        email: &str,
        password: &str,
        db: &SurrealDbClient,
    ) -> Result<Self, AppError> {
        let user: Option<User> = db
            .client
            .query(
                "SELECT * FROM user 
                WHERE email = $email 
                AND crypto::argon2::compare(password, $password)",
            )
            .bind(("email", email.to_owned()))
            .bind(("password", password.to_owned()))
            .await?
            .take(0)?;
        user.ok_or(AppError::Auth("User failed to authenticate".into()))
    }

    pub async fn find_by_email(
        email: &str,
        db: &SurrealDbClient,
    ) -> Result<Option<Self>, AppError> {
        let user: Option<User> = db
            .client
            .query("SELECT * FROM user WHERE email = $email LIMIT 1")
            .bind(("email", email.to_string()))
            .await?
            .take(0)?;

        Ok(user)
    }

    pub async fn find_by_api_key(
        api_key: &str,
        db: &SurrealDbClient,
    ) -> Result<Option<Self>, AppError> {
        let user: Option<User> = db
            .client
            .query("SELECT * FROM user WHERE api_key = $api_key LIMIT 1")
            .bind(("api_key", api_key.to_string()))
            .await?
            .take(0)?;

        Ok(user)
    }

    pub async fn set_api_key(id: &str, db: &SurrealDbClient) -> Result<String, AppError> {
        // Generate a secure random API key
        let api_key = format!("sk_{}", Uuid::new_v4().to_string().replace("-", ""));

        // Update the user record with the new API key
        let user: Option<User> = db
            .client
            .query(
                "UPDATE type::thing('user', $id) 
                SET api_key = $api_key 
                RETURN AFTER",
            )
            .bind(("id", id.to_owned()))
            .bind(("api_key", api_key.clone()))
            .await?
            .take(0)?;

        // If the user was found and updated, return the API key
        if user.is_some() {
            Ok(api_key)
        } else {
            Err(AppError::Auth("User not found".into()))
        }
    }

    pub async fn revoke_api_key(id: &str, db: &SurrealDbClient) -> Result<(), AppError> {
        let user: Option<User> = db
            .client
            .query(
                "UPDATE type::thing('user', $id) 
                SET api_key = test_string_nullish
                RETURN AFTER",
            )
            .bind(("id", id.to_owned()))
            .await?
            .take(0)?;

        if user.is_some() {
            Ok(())
        } else {
            Err(AppError::Auth("User was not found".into()))
        }
    }

    pub async fn get_knowledge_entities(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        let entities: Vec<KnowledgeEntity> = db
            .client
            .query("SELECT * FROM type::table($table) WHERE user_id = $user_id")
            .bind(("table", KnowledgeEntity::table_name()))
            .bind(("user_id", user_id.to_owned()))
            .await?
            .take(0)?;

        Ok(entities)
    }

    pub async fn get_knowledge_entities_by_type(
        user_id: &str,
        entity_type: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        let entities: Vec<KnowledgeEntity> = db
            .client
            .query("SELECT * FROM type::table($table) WHERE user_id = $user_id AND entity_type = $entity_type")
            .bind(("table", KnowledgeEntity::table_name()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("entity_type", entity_type.to_owned()))
            .await?
            .take(0)?;

        Ok(entities)
    }

    pub async fn get_entity_types(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<String>, AppError> {
        #[derive(Deserialize)]
        struct EntityTypeResponse {
            entity_type: String,
        }

        // Query to select distinct entity types for the user
        let response: Vec<EntityTypeResponse> = db
            .client
            .query("SELECT entity_type FROM type::table($table_name) WHERE user_id = $user_id GROUP BY entity_type")
            .bind(("user_id", user_id.to_owned()))
            .bind(("table_name", KnowledgeEntity::table_name()))
            .await?
            .take(0)?;

        // Extract the entity types from the response
        let entity_types: Vec<String> = response
            .into_iter()
            .map(|item| {
                let normalized = KnowledgeEntityType::from(item.entity_type.clone());
                format!("{:?}", normalized)
            })
            .collect();

        Ok(entity_types)
    }

    pub async fn get_knowledge_relationships(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<KnowledgeRelationship>, AppError> {
        let relationships: Vec<KnowledgeRelationship> = db
            .client
            .query("SELECT * FROM type::table($table) WHERE metadata.user_id = $user_id")
            .bind(("table", "relates_to"))
            .bind(("user_id", user_id.to_owned()))
            .await?
            .take(0)?;

        Ok(relationships)
    }

    pub async fn get_latest_text_contents(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<TextContent>, AppError> {
        let items: Vec<TextContent> = db
            .client
            .query("SELECT * FROM type::table($table_name) WHERE user_id = $user_id ORDER BY created_at DESC LIMIT 5")
            .bind(("user_id", user_id.to_owned()))
            .bind(("table_name", TextContent::table_name()))
            .await?
            .take(0)?;

        Ok(items)
    }

    pub async fn get_text_contents(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<TextContent>, AppError> {
        let items: Vec<TextContent> = db
            .client
            .query("SELECT * FROM type::table($table_name) WHERE user_id = $user_id ORDER BY created_at DESC")
            .bind(("user_id", user_id.to_owned()))
            .bind(("table_name", TextContent::table_name()))
            .await?
            .take(0)?;

        Ok(items)
    }

    pub async fn get_text_contents_by_category(
        user_id: &str,
        category: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<TextContent>, AppError> {
        let items: Vec<TextContent> = db
            .client
            .query("SELECT * FROM type::table($table_name) WHERE user_id = $user_id AND category = $category ORDER BY created_at DESC")
            .bind(("user_id", user_id.to_owned()))
            .bind(("category", category.to_owned()))
            .bind(("table_name", TextContent::table_name()))
            .await?
            .take(0)?;

        Ok(items)
    }

    pub async fn get_latest_knowledge_entities(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        let items: Vec<KnowledgeEntity> = db
            .client
            .query(
                "SELECT * FROM type::table($table_name) WHERE user_id = $user_id ORDER BY created_at DESC LIMIT 5",
            )
            .bind(("user_id", user_id.to_owned()))
            .bind(("table_name", KnowledgeEntity::table_name()))
            .await?
            .take(0)?;

        Ok(items)
    }
    pub async fn update_timezone(
        user_id: &str,
        timezone: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        db.query("UPDATE type::thing('user', $user_id) SET timezone = $timezone")
            .bind(("table_name", User::table_name()))
            .bind(("user_id", user_id.to_string()))
            .bind(("timezone", timezone.to_string()))
            .await?;
        Ok(())
    }

    pub async fn get_user_categories(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<String>, AppError> {
        // Query to select distinct categories for the user
        let response: Vec<CategoryResponse> = db
             .client
             .query("SELECT category FROM type::table($table_name) WHERE user_id = $user_id GROUP BY category")
             .bind(("user_id", user_id.to_owned()))
             .bind(("table_name", TextContent::table_name()))
             .await?
             .take(0)?;

        // Extract the categories from the response
        let categories: Vec<String> = response.into_iter().map(|item| item.category).collect();

        Ok(categories)
    }

    pub async fn get_and_validate_knowledge_entity(
        id: &str,
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<KnowledgeEntity, AppError> {
        let entity: KnowledgeEntity = db
            .get_item(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Entity not found".into()))?;

        if entity.user_id != user_id {
            return Err(AppError::Auth("Access denied".into()));
        }

        Ok(entity)
    }

    pub async fn get_and_validate_text_content(
        id: &str,
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<TextContent, AppError> {
        let text_content: TextContent = db
            .get_item(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Content not found".into()))?;

        if text_content.user_id != user_id {
            return Err(AppError::Auth("Access denied".into()));
        }

        Ok(text_content)
    }

    pub async fn get_user_conversations(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<Conversation>, AppError> {
        let conversations: Vec<Conversation> = db
            .client
            .query(
                "SELECT * FROM type::table($table_name) WHERE user_id = $user_id 
            ORDER BY created_at DESC",
            )
            .bind(("table_name", Conversation::table_name()))
            .bind(("user_id", user_id.to_string()))
            .await?
            .take(0)?;

        Ok(conversations)
    }

    /// Gets all active ingestion tasks for the specified user
    pub async fn get_unfinished_ingestion_tasks(
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<IngestionTask>, AppError> {
        let jobs: Vec<IngestionTask> = db
            .query(
                "SELECT * FROM type::table($table) 
                 WHERE user_id = $user_id 
                   AND (
                       state IN $active_states
                       OR (state = $failed_state AND attempts < max_attempts)
                   )
                 ORDER BY scheduled_at ASC, created_at DESC",
            )
            .bind(("table", IngestionTask::table_name()))
            .bind(("user_id", user_id.to_owned()))
            .bind((
                "active_states",
                vec![
                    TaskState::Pending.as_str(),
                    TaskState::Reserved.as_str(),
                    TaskState::Processing.as_str(),
                ],
            ))
            .bind(("failed_state", TaskState::Failed.as_str()))
            .await?
            .take(0)?;

        Ok(jobs)
    }

    /// Validate and delete job
    pub async fn validate_and_delete_job(
        id: &str,
        user_id: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        db.get_item::<IngestionTask>(id)
            .await?
            .filter(|job| job.user_id == user_id)
            .ok_or_else(|| AppError::Auth("Not authorized to delete this job".into()))?;

        db.delete_item::<IngestionTask>(id)
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    pub async fn get_knowledge_entities_by_content_category(
        user_id: &str,
        category: &str,
        db: &SurrealDbClient,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        // First, find all text content items with the specified category
        let text_contents = Self::get_text_contents_by_category(user_id, category, db).await?;

        if text_contents.is_empty() {
            return Ok(Vec::new());
        }

        // Extract source_ids
        let source_ids: Vec<String> = text_contents.iter().map(|tc| tc.id.clone()).collect();

        // Find all knowledge entities with matching source_ids
        let entities: Vec<KnowledgeEntity> = db
            .client
            .query("SELECT * FROM type::table($table) WHERE user_id = $user_id AND source_id IN $source_ids")
            .bind(("table", KnowledgeEntity::table_name()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("source_ids", source_ids))
            .await?
            .take(0)?;

        Ok(entities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::ingestion_payload::IngestionPayload;
    use crate::storage::types::ingestion_task::{IngestionTask, TaskState, MAX_ATTEMPTS};
    use std::collections::HashSet;

    // Helper function to set up a test database with SystemSettings
    async fn setup_test_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, &database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to setup the migrations");

        db
    }

    #[tokio::test]
    async fn test_user_creation() {
        // Setup test database
        let db = setup_test_db().await;

        // Create a user
        let email = "test@example.com";
        let password = "test_password";
        let timezone = "America/New_York";

        let user = User::create_new(
            email.to_string(),
            password.to_string(),
            &db,
            timezone.to_string(),
        )
        .await
        .expect("Failed to create user");

        // Verify user properties
        assert!(!user.id.is_empty());
        assert_eq!(user.email, email);
        assert_ne!(user.password, password); // Password should be hashed
        assert!(!user.anonymous);
        assert_eq!(user.timezone, timezone);

        // Verify it can be retrieved
        let retrieved: Option<User> = db
            .get_item(&user.id)
            .await
            .expect("Failed to retrieve user");
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, user.id);
        assert_eq!(retrieved.email, email);
    }

    #[tokio::test]
    async fn test_user_authentication() {
        // Setup test database
        let db = setup_test_db().await;

        // Create a user
        let email = "auth_test@example.com";
        let password = "auth_password";

        User::create_new(
            email.to_string(),
            password.to_string(),
            &db,
            "UTC".to_string(),
        )
        .await
        .expect("Failed to create user");

        // Test successful authentication
        let auth_result = User::authenticate(email, password, &db).await;
        assert!(auth_result.is_ok());

        // Test failed authentication with wrong password
        let wrong_auth = User::authenticate(email, "wrong_password", &db).await;
        assert!(wrong_auth.is_err());

        // Test failed authentication with non-existent user
        let nonexistent = User::authenticate("nonexistent@example.com", password, &db).await;
        assert!(nonexistent.is_err());
    }

    #[tokio::test]
    async fn test_get_unfinished_ingestion_tasks_filters_correctly() {
        let db = setup_test_db().await;
        let user_id = "unfinished_user";
        let other_user_id = "other_user";

        let payload = IngestionPayload::Text {
            text: "Test".to_string(),
            context: "Context".to_string(),
            category: "Category".to_string(),
            user_id: user_id.to_string(),
        };

        let created_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        db.store_item(created_task.clone())
            .await
            .expect("Failed to store created task");

        let mut processing_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        processing_task.state = TaskState::Processing;
        processing_task.attempts = 1;
        db.store_item(processing_task.clone())
            .await
            .expect("Failed to store processing task");

        let mut failed_retry_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        failed_retry_task.state = TaskState::Failed;
        failed_retry_task.attempts = 1;
        failed_retry_task.scheduled_at = chrono::Utc::now() - chrono::Duration::minutes(5);
        db.store_item(failed_retry_task.clone())
            .await
            .expect("Failed to store retryable failed task");

        let mut failed_blocked_task =
            IngestionTask::new(payload.clone(), user_id.to_string()).await;
        failed_blocked_task.state = TaskState::Failed;
        failed_blocked_task.attempts = MAX_ATTEMPTS;
        failed_blocked_task.error_message = Some("Too many failures".into());
        db.store_item(failed_blocked_task.clone())
            .await
            .expect("Failed to store blocked task");

        let mut completed_task = IngestionTask::new(payload.clone(), user_id.to_string()).await;
        completed_task.state = TaskState::Succeeded;
        db.store_item(completed_task.clone())
            .await
            .expect("Failed to store completed task");

        let other_payload = IngestionPayload::Text {
            text: "Other".to_string(),
            context: "Context".to_string(),
            category: "Category".to_string(),
            user_id: other_user_id.to_string(),
        };
        let other_task = IngestionTask::new(other_payload, other_user_id.to_string()).await;
        db.store_item(other_task)
            .await
            .expect("Failed to store other user task");

        let unfinished = User::get_unfinished_ingestion_tasks(user_id, &db)
            .await
            .expect("Failed to fetch unfinished tasks");

        let unfinished_ids: HashSet<String> =
            unfinished.iter().map(|task| task.id.clone()).collect();

        assert!(unfinished_ids.contains(&created_task.id));
        assert!(unfinished_ids.contains(&processing_task.id));
        assert!(unfinished_ids.contains(&failed_retry_task.id));
        assert!(!unfinished_ids.contains(&failed_blocked_task.id));
        assert!(!unfinished_ids.contains(&completed_task.id));
        assert_eq!(unfinished_ids.len(), 3);
    }

    #[tokio::test]
    async fn test_find_by_email() {
        // Setup test database
        let db = setup_test_db().await;

        // Create a user
        let email = "find_test@example.com";
        let password = "find_password";

        let created_user = User::create_new(
            email.to_string(),
            password.to_string(),
            &db,
            "UTC".to_string(),
        )
        .await
        .expect("Failed to create user");

        // Test finding user by email
        let found_user = User::find_by_email(email, &db)
            .await
            .expect("Error searching for user");
        assert!(found_user.is_some());
        let found_user = found_user.unwrap();
        assert_eq!(found_user.id, created_user.id);
        assert_eq!(found_user.email, email);

        // Test finding non-existent user
        let not_found = User::find_by_email("nonexistent@example.com", &db)
            .await
            .expect("Error searching for user");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_api_key_management() {
        // Setup test database
        let db = setup_test_db().await;

        // Create a user
        let email = "apikey_test@example.com";
        let password = "apikey_password";

        let user = User::create_new(
            email.to_string(),
            password.to_string(),
            &db,
            "UTC".to_string(),
        )
        .await
        .expect("Failed to create user");

        // Initially, user should have no API key
        assert!(user.api_key.is_none());

        // Generate API key
        let api_key = User::set_api_key(&user.id, &db)
            .await
            .expect("Failed to set API key");
        assert!(!api_key.is_empty());
        assert!(api_key.starts_with("sk_"));

        // Verify the API key was saved
        let updated_user: Option<User> = db
            .get_item(&user.id)
            .await
            .expect("Failed to retrieve user");
        assert!(updated_user.is_some());
        let updated_user = updated_user.unwrap();
        assert_eq!(updated_user.api_key, Some(api_key.clone()));

        // Test finding user by API key
        let found_user = User::find_by_api_key(&api_key, &db)
            .await
            .expect("Error searching by API key");
        assert!(found_user.is_some());
        let found_user = found_user.unwrap();
        assert_eq!(found_user.id, user.id);

        // Revoke API key
        User::revoke_api_key(&user.id, &db)
            .await
            .expect("Failed to revoke API key");

        // Verify API key was revoked
        let revoked_user: Option<User> = db
            .get_item(&user.id)
            .await
            .expect("Failed to retrieve user");
        assert!(revoked_user.is_some());
        let revoked_user = revoked_user.unwrap();
        assert!(revoked_user.api_key.is_none());

        // Test searching by revoked API key
        let not_found = User::find_by_api_key(&api_key, &db)
            .await
            .expect("Error searching by API key");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_password_update() {
        // Setup test database
        let db = setup_test_db().await;

        // Create a user
        let email = "pwd_test@example.com";
        let old_password = "old_password";
        let new_password = "new_password";

        User::create_new(
            email.to_string(),
            old_password.to_string(),
            &db,
            "UTC".to_string(),
        )
        .await
        .expect("Failed to create user");

        // Authenticate with old password
        let auth_result = User::authenticate(email, old_password, &db).await;
        assert!(auth_result.is_ok());

        // Update password
        User::patch_password(email, new_password, &db)
            .await
            .expect("Failed to update password");

        // Old password should no longer work
        let old_auth = User::authenticate(email, old_password, &db).await;
        assert!(old_auth.is_err());

        // New password should work
        let new_auth = User::authenticate(email, new_password, &db).await;
        assert!(new_auth.is_ok());
    }

    #[tokio::test]
    async fn test_validate_timezone() {
        // Valid timezones should be accepted as-is
        assert_eq!(validate_timezone("America/New_York"), "America/New_York");
        assert_eq!(validate_timezone("Europe/London"), "Europe/London");
        assert_eq!(validate_timezone("Asia/Tokyo"), "Asia/Tokyo");
        assert_eq!(validate_timezone("UTC"), "UTC");

        // Invalid timezones should be replaced with UTC
        assert_eq!(validate_timezone("Invalid/Timezone"), "UTC");
        assert_eq!(validate_timezone("Not_Real"), "UTC");
    }

    #[tokio::test]
    async fn test_timezone_update() {
        // Setup test database
        let db = setup_test_db().await;

        // Create user with default timezone
        let email = "timezone_test@example.com";
        let user = User::create_new(
            email.to_string(),
            "password".to_string(),
            &db,
            "UTC".to_string(),
        )
        .await
        .expect("Failed to create user");

        assert_eq!(user.timezone, "UTC");

        // Update timezone
        let new_timezone = "Europe/Paris";
        User::update_timezone(&user.id, new_timezone, &db)
            .await
            .expect("Failed to update timezone");

        // Verify timezone was updated
        let updated_user: Option<User> = db
            .get_item(&user.id)
            .await
            .expect("Failed to retrieve user");
        assert!(updated_user.is_some());
        let updated_user = updated_user.unwrap();
        assert_eq!(updated_user.timezone, new_timezone);
    }

    #[tokio::test]
    async fn test_conversations_order() {
        let db = setup_test_db().await;
        let user_id = "user_order_test";

        // Create conversations with varying updated_at timestamps
        let mut conversations = Vec::new();
        for i in 0..5 {
            let mut conv = Conversation::new(user_id.to_string(), format!("Conv {}", i));
            // Fake updated_at i minutes apart
            conv.created_at = chrono::Utc::now() - chrono::Duration::minutes(i);
            db.store_item(conv.clone())
                .await
                .expect("Failed to store conversation");
            conversations.push(conv);
        }

        // Retrieve via get_user_conversations - should be ordered by updated_at DESC
        let retrieved = User::get_user_conversations(user_id, &db)
            .await
            .expect("Failed to get conversations");

        assert_eq!(retrieved.len(), conversations.len());

        for window in retrieved.windows(2) {
            // Assert each earlier conversation has updated_at >= later conversation
            assert!(
                window[0].created_at >= window[1].created_at,
                "Conversations not ordered descending by created_at"
            );
        }

        // Check first conversation title matches the most recently updated
        let most_recent = conversations.iter().max_by_key(|c| c.created_at).unwrap();
        assert_eq!(retrieved[0].id, most_recent.id);
    }

    #[tokio::test]
    async fn test_get_latest_text_contents_returns_last_five() {
        let db = setup_test_db().await;
        let user_id = "latest_text_user";

        let mut inserted_ids = Vec::new();
        let base_time = chrono::Utc::now() - chrono::Duration::minutes(60);

        for i in 0..12 {
            let mut item = TextContent::new(
                format!("Text {}", i),
                Some(format!("Context {}", i)),
                "Category".to_string(),
                None,
                None,
                user_id.to_string(),
            );

            let timestamp = base_time + chrono::Duration::minutes(i);
            item.created_at = timestamp;
            item.updated_at = timestamp;

            db.store_item(item.clone())
                .await
                .expect("Failed to store text content");

            inserted_ids.push(item.id.clone());
        }

        let latest = User::get_latest_text_contents(user_id, &db)
            .await
            .expect("Failed to fetch latest text contents");

        assert_eq!(latest.len(), 5, "Expected exactly five items");

        let mut expected_ids = inserted_ids[inserted_ids.len() - 5..].to_vec();
        expected_ids.reverse();

        let returned_ids: Vec<String> = latest.iter().map(|item| item.id.clone()).collect();
        assert_eq!(
            returned_ids, expected_ids,
            "Latest items did not match expectation"
        );

        for window in latest.windows(2) {
            assert!(
                window[0].created_at >= window[1].created_at,
                "Results are not ordered by created_at descending"
            );
        }
    }
}
