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
    anonymous: bool
});

#[async_trait]
impl Authentication<User, String, Surreal<Any>> for User {
    async fn load_user(userid: String, pool: Option<&Surreal<Any>>) -> Result<User, anyhow::Error> {
        let pool = pool.unwrap();
        Ok(get_item::<Self>(&pool, userid.as_str()).await?.unwrap())
        // User::get_user(userid, pool)
        //     .await
        //     .ok_or_else(|| anyhow::anyhow!("Could not load user"))
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
        if let Some(_) = Self::find_by_email(&email, db).await? {
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
}
