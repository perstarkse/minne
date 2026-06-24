use super::types::{EmbeddingRecord, HasEmbedding, StoredObject};
use crate::error::AppError;
use axum_session::{SessionConfig, SessionError, SessionStore};
use axum_session_surreal::SessionSurrealPool;
use futures::Stream;
use include_dir::{Dir, include_dir};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::{ops::Deref, sync::Arc};
use surrealdb::{
    Error, Notification, Surreal,
    engine::any::{Any, connect},
    opt::auth::{Namespace, Root},
};
use surrealdb_migrations::MigrationRunner;
use tracing::debug;

/// Embedded SurrealDB project root (`migrations/`, `schemas/`, `.surrealdb`).
static MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/db");

#[derive(Clone)]
pub struct SurrealDbClient {
    pub client: Surreal<Any>,
}
#[allow(clippy::module_name_repetitions)]
pub trait ProvidesDb {
    fn db(&self) -> &Arc<SurrealDbClient>;
}

impl SurrealDbClient {
    pub async fn new(
        address: &str,
        username: &str,
        password: &str,
        namespace: &str,
        database: &str,
    ) -> Result<Self, Error> {
        let db = connect(address).await?;

        if !address.starts_with("mem://") {
            db.signin(Root { username, password }).await?;
        }

        db.use_ns(namespace).use_db(database).await?;

        Ok(SurrealDbClient { client: db })
    }

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

    pub async fn apply_migrations(&self) -> Result<(), AppError> {
        debug!("Applying migrations");
        MigrationRunner::new(&self.client)
            .load_files(&MIGRATIONS_DIR)
            .up()
            .await
            .map_err(AppError::internal)?;

        Ok(())
    }

    pub async fn store_item<T>(&self, item: T) -> Result<Option<T>, Error>
    where
        T: StoredObject + Send + Sync + 'static,
    {
        self.client
            .create((T::table_name(), item.id()))
            .content(item)
            .await
    }

    pub async fn upsert_item<T>(&self, item: T) -> Result<Option<T>, Error>
    where
        T: StoredObject + Send + Sync + 'static,
    {
        let id = item.id().to_string();
        self.client
            .upsert((T::table_name(), id))
            .content(item)
            .await
    }

    pub async fn get_all_stored_items<T>(&self) -> Result<Vec<T>, Error>
    where
        T: for<'de> StoredObject,
    {
        self.client.select(T::table_name()).await
    }

    pub async fn get_item<T>(&self, id: &str) -> Result<Option<T>, Error>
    where
        T: for<'de> StoredObject,
    {
        self.client.select((T::table_name(), id)).await
    }

    pub async fn delete_item<T>(&self, id: &str) -> Result<Option<T>, Error>
    where
        T: for<'de> StoredObject,
    {
        self.client.delete((T::table_name(), id)).await
    }

    pub async fn listen<T>(
        &self,
    ) -> Result<impl Stream<Item = Result<Notification<T>, Error>>, Error>
    where
        T: for<'de> StoredObject + std::marker::Unpin,
    {
        self.client.select(T::table_name()).live().await
    }

    /// Atomically store an entity and its embedding vector in a single
    /// SurrealDB transaction.
    ///
    /// Creates (or overwrites) the entity row and upserts the linked
    /// embedding record.  The embedding dimension is validated against
    /// `embedding_dimensions` before the query is issued.
    pub async fn store_with_embedding<E>(
        &self,
        entity: E,
        embedding: Vec<f32>,
        embedding_dimensions: usize,
    ) -> Result<(), AppError>
    where
        E: HasEmbedding + Serialize + Send + Sync + 'static,
        <E as HasEmbedding>::Embedding: Serialize + Send + Sync,
    {
        E::Embedding::validate_dimension(&embedding, embedding_dimensions)?;

        let entity_id = entity.id().to_string();
        let emb = <E as HasEmbedding>::Embedding::new(
            &entity_id,
            entity.source_id().to_string(),
            embedding,
            entity.user_id().to_string(),
            E::table_name(),
        );

        let sql = format!(
            "
            BEGIN TRANSACTION;
              CREATE type::thing('{et}', $id) CONTENT $entity;
              UPSERT type::thing('{emt}', $id) CONTENT $emb;
            COMMIT TRANSACTION;
            ",
            et = E::table_name(),
            emt = <E as HasEmbedding>::Embedding::table_name(),
        );

        self.client
            .query(sql)
            .bind(("id", entity_id))
            .bind(("entity", entity))
            .bind(("emb", emb))
            .await?
            .check()?;

        Ok(())
    }

    /// Delete all entity and embedding rows matching a given `source_id`.
    ///
    /// Runs inside a SurrealDB transaction so that entity and embedding
    /// deletes are atomic.
    pub async fn delete_by_source_id<E>(&self, source_id: &str) -> Result<(), AppError>
    where
        E: HasEmbedding,
        E::Embedding: Send + Sync,
    {
        self.client
            .query("BEGIN TRANSACTION;")
            .query(format!(
                "DELETE FROM {} WHERE source_id = $source_id;",
                E::Embedding::table_name()
            ))
            .query(format!(
                "DELETE FROM {} WHERE source_id = $source_id;",
                E::table_name()
            ))
            .query("COMMIT TRANSACTION;")
            .bind(("source_id", source_id.to_owned()))
            .await?
            .check()?;

        Ok(())
    }

    /// Vector similarity search over entities using HNSW index.
    ///
    /// Performs a cosine-similarity search against the embedding table,
    /// fetches the corresponding entity rows server-side via `FETCH`,
    /// and returns `(entity, score)` pairs ordered by descending
    /// similarity.  Orphaned embeddings (entity deleted but its
    /// embedding row remains) are logged as a warning and dropped.
    ///
    /// This is a single round-trip — SurrealDB resolves the link field
    /// (`entity_id` or `chunk_id`) inside the query engine.
    pub async fn vector_search<E, Emb>(
        &self,
        take: usize,
        query_embedding: &[f32],
        user_id: &str,
    ) -> Result<Vec<(E, f32)>, AppError>
    where
        E: StoredObject + DeserializeOwned + Clone + Send + Sync,
        Emb: EmbeddingRecord + Send + Sync,
    {
        // Generic row that works with both `entity_id` and `chunk_id` link
        // fields via `#[serde(alias)]`.  SurrealDB's `FETCH` resolves the link
        // server-side so we get the full entity in a single round-trip.
        #[derive(serde::Deserialize)]
        struct FetchRow<Ent> {
            score: f32,
            #[serde(alias = "entity_id", alias = "chunk_id")]
            entity: Option<Ent>,
        }

        let link_field = Emb::link_field();
        let sql = format!(
            r#"
            SELECT
                {link_field},
                vector::similarity::cosine(embedding, $embedding) AS score
            FROM {emb_table}
            WHERE user_id = $user_id
              AND embedding <|{take},100|> $embedding
            ORDER BY score DESC
            LIMIT {take}
            FETCH {link_field}
            "#,
            link_field = link_field,
            emb_table = Emb::table_name(),
            take = take,
        );

        let mut response = self
            .client
            .query(sql)
            .bind(("embedding", query_embedding.to_vec()))
            .bind(("user_id", user_id.to_string()))
            .await?;

        response = response.check()?;

        let rows: Vec<FetchRow<E>> = response.take(0)?;

        let mut results = Vec::with_capacity(rows.len());
        for r in rows {
            if let Some(entity) = r.entity {
                results.push((entity, r.score));
            } else {
                tracing::warn!(
                    "Vector search hit orphaned {} row with missing {link_field}",
                    Emb::table_name()
                );
            }
        }

        Ok(results)
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

    use crate::test_utils::setup_test_db;

    stored_object!(Dummy, "dummy", {
        name: String
    });

    #[tokio::test]
    async fn test_initialization_and_crud() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

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
        let db = setup_test_db().await?;

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
        let db = setup_test_db().await?;
        db.apply_migrations()
            .await
            .with_context(|| "Failed to build indexes".to_string())?;

        Ok(())
    }
}
