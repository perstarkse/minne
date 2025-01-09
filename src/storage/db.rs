use super::types::StoredObject;
use std::ops::Deref;
use surrealdb::{
    engine::any::{connect, Any},
    opt::auth::Root,
    Error, Surreal,
};

#[derive(Clone)]
pub struct SurrealDbClient {
    pub client: Surreal<Any>,
}

impl SurrealDbClient {
    /// # Initialize a new datbase client
    ///
    /// # Arguments
    ///
    /// # Returns
    /// * `SurrealDbClient` initialized
    pub async fn new(
        address: &str,
        username: &str,
        password: &str,
        namespace: &str,
        database: &str,
    ) -> Result<Self, Error> {
        let db = connect(address).await?;

        // Sign in to database
        db.signin(Root { username, password }).await?;

        // Set namespace
        db.use_ns(namespace).use_db(database).await?;

        Ok(SurrealDbClient { client: db })
    }

    pub async fn build_indexes(&self) -> Result<(), Error> {
        self.client.query("DEFINE INDEX idx_embedding_chunks ON text_chunk FIELDS embedding HNSW DIMENSION 1536").await?;
        self.client.query("DEFINE INDEX idx_embedding_entities ON knowledge_entity FIELDS embedding HNSW DIMENSION 1536").await?;

        self.client
            .query("DEFINE INDEX idx_job_status ON job FIELDS status")
            .await?;
        self.client
            .query("DEFINE INDEX idx_job_user ON job FIELDS user_id")
            .await?;
        self.client
            .query("DEFINE INDEX idx_job_created ON job FIELDS created_at")
            .await?;

        Ok(())
    }

    pub async fn rebuild_indexes(&self) -> Result<(), Error> {
        self.client
            .query("REBUILD INDEX IF EXISTS idx_embedding_chunks ON text_chunk")
            .await?;
        self.client
            .query("REBUILD INDEX IF EXISTS idx_embeddings_entities ON knowledge_entity")
            .await?;
        Ok(())
    }

    pub async fn drop_table<T>(&self) -> Result<Vec<T>, Error>
    where
        T: StoredObject + Send + Sync + 'static,
    {
        self.client.delete(T::table_name()).await
    }
}

impl Deref for SurrealDbClient {
    type Target = Surreal<Any>;

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
pub async fn store_item<T>(db_client: &Surreal<Any>, item: T) -> Result<Option<T>, Error>
where
    T: StoredObject + Send + Sync + 'static,
{
    db_client
        .create((T::table_name(), item.get_id()))
        .content(item)
        .await
}

/// Operation to retrieve all objects from a certain table, requires the struct to implement StoredObject
///
/// # Arguments
/// * `db_client` - A initialized database client
///
/// # Returns
/// * `Result` - Vec<T> or Error
pub async fn get_all_stored_items<T>(db_client: &Surreal<Any>) -> Result<Vec<T>, Error>
where
    T: for<'de> StoredObject,
{
    db_client.select(T::table_name()).await
}

/// Operation to retrieve a single object by its ID, requires the struct to implement StoredObject
///
/// # Arguments
/// * `db_client` - An initialized database client
/// * `id` - The ID of the item to retrieve
///
/// # Returns
/// * `Result<Option<T>, Error>` - The found item or Error
pub async fn get_item<T>(db_client: &Surreal<Any>, id: &str) -> Result<Option<T>, Error>
where
    T: for<'de> StoredObject,
{
    db_client.select((T::table_name(), id)).await
}

/// Operation to delete a single object by its ID, requires the struct to implement StoredObject
///
/// # Arguments
/// * `db_client` - An initialized database client
/// * `id` - The ID of the item to delete
///
/// # Returns
/// * `Result<Option<T>, Error>` - The deleted item or Error
pub async fn delete_item<T>(db_client: &Surreal<Any>, id: &str) -> Result<Option<T>, Error>
where
    T: for<'de> StoredObject,
{
    db_client.delete((T::table_name(), id)).await
}
