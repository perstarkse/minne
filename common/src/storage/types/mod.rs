#![allow(clippy::unsafe_derive_deserialize)]
#![allow(async_fn_in_trait)]
use serde::{Deserialize, Serialize};
pub mod analytics;
pub mod conversation;
pub mod file_info;
pub mod ingestion_payload;
pub mod ingestion_task;
pub mod knowledge_entity;
pub mod knowledge_entity_embedding;
pub mod knowledge_relationship;
pub mod message;
pub mod scratchpad;
pub mod system_prompts;
pub mod system_settings;
pub mod text_chunk;
pub mod text_chunk_embedding;
pub mod text_content;
pub mod user;

pub trait StoredObject: Serialize + for<'de> Deserialize<'de> {
    fn table_name() -> &'static str;
    fn id(&self) -> &str;
}

/// An entity that has an associated embedding record for vector search.
pub trait HasEmbedding: StoredObject {
    /// The embedding record type paired with this entity.
    type Embedding: EmbeddingRecord;

    fn source_id(&self) -> &str;
    fn user_id(&self) -> &str;
}

/// An embedding record linked to a `HasEmbedding` entity.
pub trait EmbeddingRecord: StoredObject {
    /// The field name in the embedding table that links back to the entity
    /// (e.g. `"entity_id"` or `"chunk_id"`). Used in FETCH and WHERE clauses.
    fn link_field() -> &'static str;

    /// The HNSW index name (e.g. `"idx_embedding_knowledge_entity_embedding"`).
    fn index_name() -> &'static str;

    fn source_id(&self) -> &str;
    fn user_id(&self) -> &str;
    fn embedding(&self) -> &[f32];

    /// Construct a new embedding record.
    ///
    /// * `id` – shared record id (same as the entity id).
    /// * `source_id` – denormalised source id for bulk deletes.
    /// * `embedding` – the embedding vector.
    /// * `user_id` – denormalised user id for query scoping.
    /// * `entity_table` – the entity's table name (used to build the link `RecordId`).
    fn new(
        id: &str,
        source_id: String,
        embedding: Vec<f32>,
        user_id: String,
        entity_table: &str,
    ) -> Self;

    /// Validate that an embedding vector matches the expected dimension.
    fn validate_dimension(embedding: &[f32], expected: usize) -> Result<(), crate::error::AppError>
    where
        Self: Sized,
    {
        if embedding.len() != expected {
            return Err(crate::error::AppError::Validation(format!(
                "embedding dimension mismatch: got {}, expected {expected}",
                embedding.len()
            )));
        }
        Ok(())
    }

    /// Recreate the HNSW vector index with a new dimension.
    ///
    /// This drops and recreates the index inside a transaction.
    async fn redefine_hnsw_index(
        db: &crate::storage::db::SurrealDbClient,
        dimension: usize,
    ) -> Result<(), crate::error::AppError>
    where
        Self: Sized,
    {
        let query = crate::storage::indexes::hnsw_index_redefine_transaction_sql(
            Self::index_name(),
            Self::table_name(),
            dimension,
        );
        db.client.query(query).await?.check()?;
        Ok(())
    }

    /// Fetch a single embedding record by its link `RecordId`.
    async fn get_by_record_id(
        db: &crate::storage::db::SurrealDbClient,
        rid: &surrealdb::RecordId,
    ) -> Result<Option<Self>, crate::error::AppError>
    where
        Self: Sized + serde::de::DeserializeOwned,
    {
        let query = format!(
            "SELECT * FROM {} WHERE {} = $rid LIMIT 1",
            Self::table_name(),
            Self::link_field(),
        );
        let mut result = db.client.query(query).bind(("rid", rid.clone())).await?;
        Ok(result.take(0)?)
    }

    /// Delete an embedding record by its link `RecordId`.
    async fn delete_by_record_id(
        db: &crate::storage::db::SurrealDbClient,
        rid: &surrealdb::RecordId,
    ) -> Result<(), crate::error::AppError>
    where
        Self: Sized,
    {
        let query = format!(
            "DELETE FROM {} WHERE {} = $rid",
            Self::table_name(),
            Self::link_field(),
        );
        db.client
            .query(query)
            .bind(("rid", rid.clone()))
            .await?
            .check()?;
        Ok(())
    }

    /// Delete all embedding records with a given `source_id`.
    async fn delete_by_source_id(
        source_id: &str,
        db: &crate::storage::db::SurrealDbClient,
    ) -> Result<(), crate::error::AppError>
    where
        Self: Sized,
    {
        let query = format!(
            "DELETE FROM {} WHERE source_id = $source_id",
            Self::table_name(),
        );
        db.client
            .query(query)
            .bind(("source_id", source_id.to_owned()))
            .await?
            .check()?;
        Ok(())
    }
}

#[macro_export]
macro_rules! stored_object {
    ($(#[$struct_attr:meta])* $name:ident, $table:expr, {$($(#[$field_attr:meta])* $field:ident: $ty:ty),*}) => {
        use serde::{Deserialize, Serialize};
        use $crate::storage::types::StoredObject;
        #[allow(unused_imports)]
        use $crate::utils::serde_helpers::{
            deserialize_flexible_id, serialize_datetime, deserialize_datetime,
            serialize_option_datetime, deserialize_option_datetime,
        };
        use chrono::{DateTime, Utc };

        $(#[$struct_attr])*
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        pub struct $name {
            #[serde(deserialize_with = "deserialize_flexible_id")]
            pub id: String,
            #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime", default)]
            pub created_at: DateTime<Utc>,
            #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime", default)]
            pub updated_at: DateTime<Utc>,
            $( $(#[$field_attr])* pub $field: $ty),*
        }

        impl StoredObject for $name {
            fn table_name() -> &'static str {
                $table
            }

            fn id(&self) -> &str {
                &self.id
            }
        }
    };
}
