use crate::{
    error::AppError,
    storage::db::{get_item, SurrealDbClient},
    stored_object,
};
use axum_session_auth::Authentication;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

use super::{
    knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
    system_settings::SystemSettings, text_content::TextContent,
};

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
        Ok(get_item::<Self>(db, userid.as_str()).await?.unwrap())
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
    use chrono_tz::Tz;

    // Check if it's a valid IANA timezone identifier
    match input.parse::<Tz>() {
        Ok(_) => input.to_owned(),
        Err(_) => {
            tracing::warn!("Invalid timezone '{}' received, defaulting to UTC", input);
            "UTC".to_owned()
        }
    }
}

impl User {
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
                admin = $count < 1,  // Changed from == 0 to < 1
                anonymous = false,
                created_at = $created_at,
                updated_at = $updated_at,
                timezone = $timezone",
            )
            .bind(("table", "user"))
            .bind(("id", id))
            .bind(("email", email))
            .bind(("password", password))
            .bind(("created_at", now))
            .bind(("updated_at", now))
            .bind(("timezone", validated_tz))
            .await?
            .take(1)?;

        user.ok_or(AppError::Auth("User failed to create".into()))
    }

    pub async fn authenticate(
        email: String,
        password: String,
        db: &SurrealDbClient,
    ) -> Result<Self, AppError> {
        let user: Option<User> = db
            .client
            .query(
                "SELECT * FROM user 
                WHERE email = $email 
                AND crypto::argon2::compare(password, $password)",
            )
            .bind(("email", email))
            .bind(("password", password))
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
                SET api_key = NULL 
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
        let entity: KnowledgeEntity = get_item(db, &id)
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
        let text_content: TextContent = get_item(db, &id)
            .await?
            .ok_or_else(|| AppError::NotFound("Content not found".into()))?;

        if text_content.user_id != user_id {
            return Err(AppError::Auth("Access denied".into()));
        }

        Ok(text_content)
    }
}
