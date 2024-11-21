use crate::error::ProcessingError;

use super::types::StoredObject;
use std::ops::Deref;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Error, Surreal,
};

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

/// Operation to store a object in SurrealDB, requires the struct to implement StoredObject
///
/// # Arguments
/// * `db_client` - A initialized database client
/// * `item` - The item to be stored
///
/// # Returns
/// * `Result` - Item or Error
pub async fn store_item<T>(
    db_client: &Surreal<Client>,
    item: T,
) -> Result<Option<T>, ProcessingError>
where
    T: StoredObject + Send + Sync + 'static,
{
    db_client
        .create((T::table_name(), item.get_id()))
        .content(item)
        .await
        .map_err(ProcessingError::from)
}
