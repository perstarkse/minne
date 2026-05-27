use super::types::StoredObject;
use crate::error::AppError;
use axum_session::{SessionConfig, SessionError, SessionStore};
use axum_session_surreal::SessionSurrealPool;
use futures::Stream;
use include_dir::{include_dir, Dir};
use std::{ops::Deref, sync::Arc};
use surrealdb::{
    engine::any::{connect, Any},
    opt::auth::{Namespace, Root},
    Error, Notification, Surreal,
};
use surrealdb_migrations::MigrationRunner;
use tracing::debug;

/// Embedded SurrealDB migration directory packaged with the crate.
static MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/");

#[derive(Clone)]
pub struct SurrealDbClient {
    pub client: Surreal<Any>,
}
#[allow(clippy::module_name_repetitions)]
pub trait ProvidesDb {
    fn db(&self) -> &Arc<SurrealDbClient>;
}

impl SurrealDbClient {
    /// Initialize a new database client.
    ///
    /// # Arguments
    ///
    /// * `address` — Database connection string (e.g. `ws://localhost:8000` or `mem://`).
    /// * `username` — Root username for authentication.
    /// * `password` — Root password for authentication.
    /// * `namespace` — SurrealDB namespace to use.
    /// * `database` — SurrealDB database to use.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the connection, authentication, or namespace/database selection fails.
    /// In-memory (`mem://`) connections skip authentication.
    pub async fn new(
        address: &str,
        username: &str,
        password: &str,
        namespace: &str,
        database: &str,
    ) -> Result<Self, Error> {
        let db = connect(address).await?;

        // Skip sign-in for in-memory engine (no auth support)
        if !address.starts_with("mem://") {
            db.signin(Root { username, password }).await?;
        }

        // Set namespace
        db.use_ns(namespace).use_db(database).await?;

        Ok(SurrealDbClient { client: db })
    }

    /// Initialize a new database client using namespace-level authentication.
    ///
    /// # Arguments
    ///
    /// * `address` — Database connection string.
    /// * `namespace` — SurrealDB namespace to use (also used for auth).
    /// * `username` — Namespace username for authentication.
    /// * `password` — Namespace password for authentication.
    /// * `database` — SurrealDB database to use.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the connection, namespace authentication, or namespace/database selection fails.
    pub async fn new_with_namespace_user(
        address: &str,
        namespace: &str,
        username: &str,
        password: &str,
        database: &str,
    ) -> Result<Self, Error> {
        let db = connect(address).await?;
        db.signin(Namespace {
            namespace,
            username,
            password,
        })
        .await?;
        db.use_ns(namespace).use_db(database).await?;
        Ok(SurrealDbClient { client: db })
    }

    /// Create an Axum session store backed by SurrealDB.
    ///
    /// # Errors
    ///
    /// Returns `SessionError` if the session store configuration or table creation fails.
    pub async fn create_session_store(
        &self,
    ) -> Result<SessionStore<SessionSurrealPool<Any>>, SessionError> {
        debug!("Creating session store");
        SessionStore::new(
            Some(self.client.clone().into()),
            SessionConfig::default()
                .with_table_name("session")
                .with_secure(true),
        )
        .await
    }

    /// Applies all pending database migrations found in the embedded MIGRATIONS_DIR.
    ///
    /// This function should be called during application startup, after connecting to
    /// the database and selecting the appropriate namespace and database, but before
    /// the application starts performing operations that rely on the schema.
    ///
    /// # Errors
    ///
    /// Returns `AppError::InternalError` if the migration runner fails to apply any migration.
    pub async fn apply_migrations(&self) -> Result<(), AppError> {
        debug!("Applying migrations");
        MigrationRunner::new(&self.client)
            .load_files(&MIGRATIONS_DIR)
            .up()
            .await
            .map_err(|e| AppError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Store an object in SurrealDB.
    ///
    /// # Arguments
    ///
    /// * `item` — The item to store. Must implement `StoredObject`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the database create operation fails.
    pub async fn store_item<T>(&self, item: T) -> Result<Option<T>, Error>
    where
        T: StoredObject + Send + Sync + 'static,
    {
        self.client
            .create((T::table_name(), item.get_id()))
            .content(item)
            .await
    }

    /// Upsert an object in SurrealDB, replacing any existing record with the same ID.
    ///
    /// Useful for idempotent ingestion flows.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the database upsert operation fails.
    pub async fn upsert_item<T>(&self, item: T) -> Result<Option<T>, Error>
    where
        T: StoredObject + Send + Sync + 'static,
    {
        let id = item.get_id().to_string();
        self.client
            .upsert((T::table_name(), id))
            .content(item)
            .await
    }

    /// Retrieve all objects from a table.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the database select operation fails.
    pub async fn get_all_stored_items<T>(&self) -> Result<Vec<T>, Error>
    where
        T: for<'de> StoredObject,
    {
        self.client.select(T::table_name()).await
    }

    /// Retrieve a single object by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` — The ID of the item to retrieve.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the database select operation fails.
    /// Returns `Ok(None)` if no record with the given ID exists.
    pub async fn get_item<T>(&self, id: &str) -> Result<Option<T>, Error>
    where
        T: for<'de> StoredObject,
    {
        self.client.select((T::table_name(), id)).await
    }

    /// Delete a single object by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` — The ID of the item to delete.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the database delete operation fails.
    /// Returns `Ok(None)` if no record with the given ID exists.
    pub async fn delete_item<T>(&self, id: &str) -> Result<Option<T>, Error>
    where
        T: for<'de> StoredObject,
    {
        self.client.delete((T::table_name(), id)).await
    }

    /// Listen to a table for real-time updates via a live query stream.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the database live query subscription fails.
    pub async fn listen<T>(
        &self,
    ) -> Result<impl Stream<Item = Result<Notification<T>, Error>>, Error>
    where
        T: for<'de> StoredObject + std::marker::Unpin,
    {
        self.client.select(T::table_name()).live().await
    }
}

impl Deref for SurrealDbClient {
    type Target = Surreal<Any>;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl SurrealDbClient {
    /// Create an in-memory SurrealDB client for testing.
    pub async fn memory(namespace: &str, database: &str) -> Result<Self, Error> {
        let db = connect("mem://").await?;

        db.use_ns(namespace).use_db(database).await?;

        Ok(SurrealDbClient { client: db })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use crate::stored_object;
    use anyhow::{self, Context};

    use super::*;
    use uuid::Uuid;

    stored_object!(Dummy, "dummy", {
        name: String
    });

    #[tokio::test]
    async fn test_initialization_and_crud() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        db.apply_migrations()
            .await
            .with_context(|| "Failed to initialize schema".to_string())?;

        let dummy = Dummy {
            id: "abc".to_string(),
            name: "first".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let stored = db
            .store_item(dummy.clone())
            .await
            .with_context(|| "Failed to store".to_string())?;
        assert!(stored.is_some());

        let fetched = db
            .get_item::<Dummy>(&dummy.id)
            .await
            .with_context(|| "Failed to fetch".to_string())?;
        assert_eq!(fetched, Some(dummy.clone()));

        let all = db
            .get_all_stored_items::<Dummy>()
            .await
            .with_context(|| "Failed to fetch all".to_string())?;
        assert!(all.contains(&dummy));

        let deleted = db
            .delete_item::<Dummy>(&dummy.id)
            .await
            .with_context(|| "Failed to delete".to_string())?;
        assert_eq!(deleted, Some(dummy));

        let fetch_post = db
            .get_item::<Dummy>("abc")
            .await
            .with_context(|| "Failed fetch post delete".to_string())?;
        assert!(fetch_post.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn upsert_item_overwrites_existing_records() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        db.apply_migrations()
            .await
            .with_context(|| "Failed to initialize schema".to_string())?;

        let mut dummy = Dummy {
            id: "abc".to_string(),
            name: "first".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.store_item(dummy.clone())
            .await
            .with_context(|| "Failed to store initial record".to_string())?;

        dummy.name = "updated".to_string();
        let upserted = db
            .upsert_item(dummy.clone())
            .await
            .with_context(|| "Failed to upsert record".to_string())?;
        assert!(upserted.is_some());

        let fetched: Option<Dummy> = db
            .get_item(&dummy.id)
            .await
            .with_context(|| "fetch after upsert".to_string())?;
        let fetched =
            fetched.ok_or_else(|| anyhow::anyhow!("Expected record to exist after upsert"))?;
        assert_eq!(fetched.name, "updated");

        let new_record = Dummy {
            id: "def".to_string(),
            name: "brand-new".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.upsert_item(new_record.clone())
            .await
            .with_context(|| "Failed to upsert new record".to_string())?;

        let fetched_new: Option<Dummy> = db
            .get_item(&new_record.id)
            .await
            .with_context(|| "fetch inserted via upsert".to_string())?;
        assert_eq!(fetched_new, Some(new_record));

        Ok(())
    }

    #[tokio::test]
    async fn test_applying_migrations() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        db.apply_migrations()
            .await
            .with_context(|| "Failed to build indexes".to_string())?;

        Ok(())
    }
}
