// use crate::{error::ApiError, server::routes::auth::SignupParams, storage::db::SurrealDbClient};
// use axum::async_trait;
// use axum_session_auth::Authentication;
// use serde::{Deserialize, Serialize};
// use surrealdb::{
//     engine::any::Any,
//     opt::auth::{Database, Namespace, Record},
//     Object, Surreal,
// };
// use tracing::info;
// use uuid::Uuid;

// #[derive(Deserialize, Serialize)]
// pub struct AuthParams {
//     email: String,
//     password: String,
//     user_id: String,
// }

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct User {
//     pub user_id: String,
//     pub email: String,
//     #[serde(default)]
//     pub anonymous: bool,
// }

// impl Default for User {
//     fn default() -> Self {
//         Self {
//             user_id: "user:guest".into(),
//             email: "guest@example.com".into(),
//             anonymous: true,
//         }
//     }
// }

// #[async_trait]
// impl Authentication<User, i64, Surreal<Any>> for User {
//     async fn load_user(userid: i64, pool: Option<&Surreal<Any>>) -> Result<User, anyhow::Error> {
//         let pool = pool.unwrap();
//         User::get_user(userid, pool)
//             .await
//             .ok_or_else(|| anyhow::anyhow!("Could not load user"))
//     }

//     fn is_authenticated(&self) -> bool {
//         !self.anonymous
//     }

//     fn is_active(&self) -> bool {
//         !self.anonymous
//     }

//     fn is_anonymous(&self) -> bool {
//         self.anonymous
//     }
// }

// impl User {
//     // pub async fn get_user_by_email(
//     //     email: &str,
//     //     db: &SurrealDbClient,
//     // ) -> Result<Option<Self>, ApiError> {
//     //     info!("First, let's see what records exist");
//     //     let debug_query: Vec<User> = db.select("users").await?;
//     //     // let debug_query: Vec<User> = db.client.query("SELECT * FROM user").await?.take(0)?;
//     //     info!("All users in database: {:?}", debug_query);

//     //     // let tables: Vec<String> = db.client.query("INFO FOR DB").await?.take(0)?;
//     //     // info!("Available tables: {:?}", tables);

//     //     // Modified query to match exactly how the record is stored
//     //     let user: Option<User> = db
//     //         .client
//     //         .query("SELECT * FROM user WHERE email = $email LIMIT 1")
//     //         .bind(("email", email.to_string()))
//     //         .await?
//     //         .take(0)?;

//     //     info!("Found user: {:?}", user);

//     //     Ok(user)
//     // }

//     pub async fn get_user(id: i64, pool: &Surreal<Any>) -> Option<Self> {
//         let user: Option<User> = pool
//             .query("SELECT * FROM user WHERE user_id = $user_id")
//             .bind(("user_id", format!("user:{}", id)))
//             .await
//             .ok()?
//             .take(0)
//             .ok()?;

//         user
//     }

//     pub async fn signin(params: SignupParams, db: &SurrealDbClient) -> Result<(), ApiError> {
//         info!("Trying to sign in");
//         let result = db
//             .client
//             .signin(Record {
//                 access: "account",
//                 namespace: "test",

//                 database: "test",
//                 params: SignupParams {
//                     email: params.email,
//                     password: params.password,
//                 },
//             })
//             .await?;

//         info!("{:?}", result.into_insecure_token());
//         Ok(())
//     }

//     pub async fn signup(params: SignupParams, db: &SurrealDbClient) -> Result<Self, ApiError> {
//         // First check if user already exists
//         if let Some(_) = Self::get_user_by_email(&params.email, db).await? {
//             return Err(ApiError::UserAlreadyExists);
//         }

//         // Use SurrealDB's built-in signup
//         let signup_response = db
//             .client
//             .signup(Record {
//                 access: "account",
//                 namespace: "test",
//                 database: "test",
//                 params: AuthParams {
//                     email: params.email.clone(),
//                     password: params.password.clone(),
//                     user_id: Uuid::new_v4().to_string(),
//                 },
//             })
//             .await?;

//         info!("Signup response: {:?}", signup_response);

//         // Wait a moment to ensure the record is created
//         tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

//         Self::signin(params, db).await?;
//         // Fetch the created user
//         // let user = Self::get_user_by_email(&params.email, db)
//         //     .await?
//         //     .ok_or(ApiError::UserNotFound)?;

//         Ok(User::default())
//     }
// }
