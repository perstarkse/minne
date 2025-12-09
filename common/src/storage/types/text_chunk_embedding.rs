use surrealdb::RecordId;

use crate::storage::types::text_chunk::TextChunk;
use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

stored_object!(TextChunkEmbedding, "text_chunk_embedding", {
    /// Record link to the owning text_chunk
    chunk_id: RecordId,
    /// Denormalized source id for bulk deletes
    source_id: String,
    /// Embedding vector
    embedding: Vec<f32>,
    /// Denormalized user id (for scoping + permissions)
    user_id: String
});

impl TextChunkEmbedding {
    /// Recreate the HNSW index with a new embedding dimension.
    ///
    /// This is useful when the embedding length changes; Surreal requires the
    /// index definition to be recreated with the updated dimension.
    pub async fn redefine_hnsw_index(
        db: &SurrealDbClient,
        dimension: usize,
    ) -> Result<(), AppError> {
        let query = format!(
            "BEGIN TRANSACTION;
             REMOVE INDEX IF EXISTS idx_embedding_text_chunk_embedding ON TABLE {table};
             DEFINE INDEX idx_embedding_text_chunk_embedding ON TABLE {table} FIELDS embedding HNSW DIMENSION {dimension};
             COMMIT TRANSACTION;",
            table = Self::table_name(),
        );

        let res = db.client.query(query).await.map_err(AppError::Database)?;
        res.check().map_err(AppError::Database)?;

        Ok(())
    }

    /// Create a new text chunk embedding
    ///
    /// `chunk_id` is the **key** part of the text_chunk id (e.g. the UUID),
    /// not "text_chunk:uuid".
    pub fn new(chunk_id: &str, source_id: String, embedding: Vec<f32>, user_id: String) -> Self {
        let now = Utc::now();

        Self {
            // NOTE: `stored_object!` macro defines `id` as `String`
            id: uuid::Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            // Create a record<text_chunk> link: text_chunk:<chunk_id>
            chunk_id: RecordId::from_table_key(TextChunk::table_name(), chunk_id),
            source_id,
            embedding,
            user_id,
        }
    }

    /// Get a single embedding by its chunk RecordId
    pub async fn get_by_chunk_id(
        chunk_id: &RecordId,
        db: &SurrealDbClient,
    ) -> Result<Option<Self>, AppError> {
        let query = format!(
            "SELECT * FROM {} WHERE chunk_id = $chunk_id LIMIT 1",
            Self::table_name()
        );

        let mut result = db
            .client
            .query(query)
            .bind(("chunk_id", chunk_id.clone()))
            .await
            .map_err(AppError::Database)?;

        let embeddings: Vec<Self> = result.take(0).map_err(AppError::Database)?;

        Ok(embeddings.into_iter().next())
    }

    /// Delete embeddings for a given chunk RecordId
    pub async fn delete_by_chunk_id(
        chunk_id: &RecordId,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE FROM {} WHERE chunk_id = $chunk_id",
            Self::table_name()
        );

        db.client
            .query(query)
            .bind(("chunk_id", chunk_id.clone()))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        Ok(())
    }

    /// Delete all embeddings that belong to chunks with a given `source_id`
    ///
    /// This uses a subquery to the `text_chunk` table:
    ///
    /// DELETE FROM text_chunk_embedding
    /// WHERE chunk_id IN (SELECT id FROM text_chunk WHERE source_id = $source_id)
    pub async fn delete_by_source_id(
        source_id: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        #[allow(clippy::missing_docs_in_private_items)]
        #[derive(Deserialize)]
        struct IdRow {
            id: RecordId,
        }
        let ids_query = format!(
            "SELECT id FROM {} WHERE source_id = $source_id",
            TextChunk::table_name()
        );
        let mut res = db
            .client
            .query(ids_query)
            .bind(("source_id", source_id.to_owned()))
            .await
            .map_err(AppError::Database)?;
        let ids: Vec<IdRow> = res.take(0).map_err(AppError::Database)?;

        if ids.is_empty() {
            return Ok(());
        }
        let delete_query = format!(
            "DELETE FROM {} WHERE chunk_id IN $chunk_ids",
            Self::table_name()
        );
        db.client
            .query(delete_query)
            .bind((
                "chunk_ids",
                ids.into_iter().map(|row| row.id).collect::<Vec<_>>(),
            ))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::SurrealDbClient;
    use surrealdb::Value as SurrealValue;
    use uuid::Uuid;

    /// Helper to create an in-memory DB and apply migrations
    async fn setup_test_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, &database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        db
    }

    /// Helper: create a text_chunk with a known key, return its RecordId
    async fn create_text_chunk_with_id(
        db: &SurrealDbClient,
        key: &str,
        source_id: &str,
        user_id: &str,
    ) -> RecordId {
        let chunk = TextChunk {
            id: key.to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_id: source_id.to_owned(),
            chunk: "Some test chunk text".to_owned(),
            user_id: user_id.to_owned(),
        };

        db.store_item(chunk)
            .await
            .expect("Failed to create text_chunk");

        RecordId::from_table_key(TextChunk::table_name(), key)
    }

    #[tokio::test]
    async fn test_create_and_get_by_chunk_id() {
        let db = setup_test_db().await;

        let user_id = "user_a";
        let chunk_key = "chunk-123";
        let source_id = "source-1";

        // 1) Create a text_chunk with a known key
        let chunk_rid = create_text_chunk_with_id(&db, chunk_key, source_id, user_id).await;

        // 2) Create and store an embedding for that chunk
        let embedding_vec = vec![0.1_f32, 0.2, 0.3];
        let emb = TextChunkEmbedding::new(
            chunk_key,
            source_id.to_string(),
            embedding_vec.clone(),
            user_id.to_string(),
        );

        TextChunkEmbedding::redefine_hnsw_index(&db, emb.embedding.len())
            .await
            .expect("Failed to redefine index length");

        let _: Option<TextChunkEmbedding> = db
            .client
            .create(TextChunkEmbedding::table_name())
            .content(emb)
            .await
            .expect("Failed to store embedding")
            .take()
            .expect("Failed to deserialize stored embedding");

        // 3) Fetch it via get_by_chunk_id
        let fetched = TextChunkEmbedding::get_by_chunk_id(&chunk_rid, &db)
            .await
            .expect("Failed to get embedding by chunk_id");

        assert!(fetched.is_some(), "Expected an embedding to be found");
        let fetched = fetched.unwrap();

        assert_eq!(fetched.user_id, user_id);
        assert_eq!(fetched.chunk_id, chunk_rid);
        assert_eq!(fetched.embedding, embedding_vec);
    }

    #[tokio::test]
    async fn test_delete_by_chunk_id() {
        let db = setup_test_db().await;

        let user_id = "user_b";
        let chunk_key = "chunk-delete";
        let source_id = "source-del";

        let chunk_rid = create_text_chunk_with_id(&db, chunk_key, source_id, user_id).await;

        let emb = TextChunkEmbedding::new(
            chunk_key,
            source_id.to_string(),
            vec![0.4_f32, 0.5, 0.6],
            user_id.to_string(),
        );

        TextChunkEmbedding::redefine_hnsw_index(&db, emb.embedding.len())
            .await
            .expect("Failed to redefine index length");

        let _: Option<TextChunkEmbedding> = db
            .client
            .create(TextChunkEmbedding::table_name())
            .content(emb)
            .await
            .expect("Failed to store embedding")
            .take()
            .expect("Failed to deserialize stored embedding");

        // Ensure it exists
        let existing = TextChunkEmbedding::get_by_chunk_id(&chunk_rid, &db)
            .await
            .expect("Failed to get embedding before delete");
        assert!(existing.is_some(), "Embedding should exist before delete");

        // Delete by chunk_id
        TextChunkEmbedding::delete_by_chunk_id(&chunk_rid, &db)
            .await
            .expect("Failed to delete by chunk_id");

        // Ensure it no longer exists
        let after = TextChunkEmbedding::get_by_chunk_id(&chunk_rid, &db)
            .await
            .expect("Failed to get embedding after delete");
        assert!(after.is_none(), "Embedding should have been deleted");
    }

    #[tokio::test]
    async fn test_delete_by_source_id() {
        let db = setup_test_db().await;

        let user_id = "user_c";
        let source_id = "shared-source";
        let other_source = "other-source";

        // Two chunks with the same source_id
        let chunk1_rid = create_text_chunk_with_id(&db, "chunk-s1", source_id, user_id).await;
        let chunk2_rid = create_text_chunk_with_id(&db, "chunk-s2", source_id, user_id).await;

        // One chunk with a different source_id
        let chunk_other_rid =
            create_text_chunk_with_id(&db, "chunk-other", other_source, user_id).await;

        // Create embeddings for all three
        let emb1 = TextChunkEmbedding::new(
            "chunk-s1",
            source_id.to_string(),
            vec![0.1],
            user_id.to_string(),
        );
        let emb2 = TextChunkEmbedding::new(
            "chunk-s2",
            source_id.to_string(),
            vec![0.2],
            user_id.to_string(),
        );
        let emb3 = TextChunkEmbedding::new(
            "chunk-other",
            other_source.to_string(),
            vec![0.3],
            user_id.to_string(),
        );

        // Update length on index
        TextChunkEmbedding::redefine_hnsw_index(&db, emb1.embedding.len())
            .await
            .expect("Failed to redefine index length");

        for emb in [emb1, emb2, emb3] {
            let _: Option<TextChunkEmbedding> = db
                .client
                .create(TextChunkEmbedding::table_name())
                .content(emb)
                .await
                .expect("Failed to store embedding")
                .take()
                .expect("Failed to deserialize stored embedding");
        }

        // Sanity check: they all exist
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk1_rid, &db)
            .await
            .unwrap()
            .is_some());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk2_rid, &db)
            .await
            .unwrap()
            .is_some());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk_other_rid, &db)
            .await
            .unwrap()
            .is_some());

        // Delete embeddings by source_id (shared-source)
        TextChunkEmbedding::delete_by_source_id(source_id, &db)
            .await
            .expect("Failed to delete by source_id");

        // Chunks from shared-source should have no embeddings
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk1_rid, &db)
            .await
            .unwrap()
            .is_none());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk2_rid, &db)
            .await
            .unwrap()
            .is_none());

        // The other chunk should still have its embedding
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk_other_rid, &db)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_redefine_hnsw_index_updates_dimension() {
        let db = setup_test_db().await;

        // Change the index dimension from default (1536) to a smaller test value.
        TextChunkEmbedding::redefine_hnsw_index(&db, 8)
            .await
            .expect("failed to redefine index");

        let mut info_res = db
            .client
            .query("INFO FOR TABLE text_chunk_embedding;")
            .await
            .expect("info query failed");
        let info: SurrealValue = info_res.take(0).expect("failed to take info result");
        let info_json: serde_json::Value =
            serde_json::to_value(info).expect("failed to convert info to json");
        let idx_sql = info_json["Object"]["indexes"]["Object"]
            ["idx_embedding_text_chunk_embedding"]["Strand"]
            .as_str()
            .unwrap_or_default();

        assert!(
            idx_sql.contains("DIMENSION 8"),
            "expected index definition to contain new dimension, got: {idx_sql}"
        );
    }

    #[tokio::test]
    async fn test_redefine_hnsw_index_is_idempotent() {
        let db = setup_test_db().await;

        TextChunkEmbedding::redefine_hnsw_index(&db, 4)
            .await
            .expect("first redefine failed");
        TextChunkEmbedding::redefine_hnsw_index(&db, 4)
            .await
            .expect("second redefine failed");

        let mut info_res = db
            .client
            .query("INFO FOR TABLE text_chunk_embedding;")
            .await
            .expect("info query failed");
        let info: SurrealValue = info_res.take(0).expect("failed to take info result");
        let info_json: serde_json::Value =
            serde_json::to_value(info).expect("failed to convert info to json");
        let idx_sql = info_json["Object"]["indexes"]["Object"]
            ["idx_embedding_text_chunk_embedding"]["Strand"]
            .as_str()
            .unwrap_or_default();

        assert!(
            idx_sql.contains("DIMENSION 4"),
            "expected index definition to retain dimension 4, got: {idx_sql}"
        );
    }
}
