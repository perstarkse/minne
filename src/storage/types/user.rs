use crate::{
    error::ApiError,
    storage::db::{get_item, SurrealDbClient},
    stored_object,
};
use axum_session_auth::Authentication;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

stored_object!(User, "user", {
    email: String,
    password: String,
    anonymous: bool,
    api_key: Option<String>
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

impl User {
    pub async fn create_new(
        email: String,
        password: String,
        db: &SurrealDbClient,
    ) -> Result<Self, ApiError> {
        // Check if user exists
        if (Self::find_by_email(&email, db).await?).is_some() {
            return Err(ApiError::UserAlreadyExists);
        }

        let id = Uuid::new_v4().to_string();
        let user: Option<User> = db
            .client
            .query(
                "CREATE type::thing('user', $id) SET 
                email = $email, 
                password = crypto::argon2::generate($password),
                anonymous = false",
            )
            .bind(("id", id))
            .bind(("email", email))
            .bind(("password", password))
            .await?
            .take(0)?;

        user.ok_or(ApiError::UserAlreadyExists)
    }

    pub async fn authenticate(
        email: String,
        password: String,
        db: &SurrealDbClient,
    ) -> Result<Self, ApiError> {
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
        user.ok_or(ApiError::UserAlreadyExists)
    }

    pub async fn find_by_email(
        email: &str,
        db: &SurrealDbClient,
    ) -> Result<Option<Self>, ApiError> {
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
    ) -> Result<Option<Self>, ApiError> {
        let user: Option<User> = db
            .client
            .query("SELECT * FROM user WHERE api_key = $api_key LIMIT 1")
            .bind(("api_key", api_key.to_string()))
            .await?
            .take(0)?;

        Ok(user)
    }

    pub async fn set_api_key(id: &str, db: &SurrealDbClient) -> Result<String, ApiError> {
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
            Err(ApiError::UserNotFound)
        }
    }
    pub async fn reset_api_key(id: &str, db: &SurrealDbClient) -> Result<String, ApiError> {
        // Simply call set_api_key to generate and set a new key
        Self::set_api_key(id, db).await
    }

    pub async fn revoke_api_key(id: &str, db: &SurrealDbClient) -> Result<(), ApiError> {
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
            Err(ApiError::UserNotFound)
        }
    }
}
