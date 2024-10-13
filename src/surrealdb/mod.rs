use surrealdb::{engine::remote::ws::{Client, Ws}, opt::auth::Root, Surreal};
use thiserror::Error;

pub mod document;
pub mod graph;

pub struct SurrealDbClient {
    pub client: Surreal<Client>,
}

#[derive(Error, Debug)]
pub enum SurrealError {
    #[error("SurrealDb error: {0}")]
    SurrealDbError(#[from] surrealdb::Error),

    // Add more error variants as needed.
}


impl SurrealDbClient {
    /// # Initialize a new datbase client
    ///
    /// # Arguments
    ///
    /// # Returns
    /// * `SurrealDbClient` initialized
    pub async fn new() -> Result<Self, SurrealError> {
        let db = Surreal::new::<Ws>("127.0.0.1:8000").await?;

        // Sign in to database
        db.signin(Root{username: "root_user", password: "root_password"}).await?;

        // Set namespace
        db.use_ns("test").use_db("test").await?;

        Ok(SurrealDbClient { client: db })
    }
}
