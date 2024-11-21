use std::ops::Deref;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Error, Surreal,
};

use crate::storage::types::StoredObject;

#[derive(Clone)]
pub struct SurrealDbClient {
    pub client: Surreal<Client>,
}

impl SurrealDbClient {
    /// # Initialize a new datbase client
    ///
    /// # Arguments
    ///
    /// # Returns
    /// * `SurrealDbClient` initialized
    pub async fn new() -> Result<Self, Error> {
        let db = Surreal::new::<Ws>("127.0.0.1:8000").await?;

        // Sign in to database
        db.signin(Root {
            username: "root_user",
            password: "root_password",
        })
        .await?;

        // Set namespace
        db.use_ns("test").use_db("test").await?;

        Ok(SurrealDbClient { client: db })
    }

    pub async fn rebuild_indexes(&self) -> Result<(), Error> {
        self.client
            .query("REBUILD INDEX IF EXISTS idx_embedding ON text_chunk")
            .await?;
        self.client
            .query("REBUILD INDEX IF EXISTS embeddings ON knowledge_entity")
            .await?;
        Ok(())
    }

    pub async fn drop_table<T>(&self) -> Result<(), Error>
    where
        T: StoredObject + Send + Sync + 'static,
    {
        let _deleted: Vec<T> = self.client.delete(T::table_name()).await?;
        Ok(())
    }
}

impl Deref for SurrealDbClient {
    type Target = Surreal<Client>;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}
