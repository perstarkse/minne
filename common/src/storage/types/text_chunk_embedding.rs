use surrealdb::RecordId;

use crate::storage::types::text_chunk::TextChunk;
use crate::{
    error::AppError,
    storage::{db::SurrealDbClient, indexes::hnsw_index_redefine_transaction_sql},
    stored_object,
};

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
        let query = hnsw_index_redefine_transaction_sql(
            "idx_embedding_text_chunk_embedding",
            Self::table_name(),
            dimension,
        );

        let res = db.client.query(query).await.map_err(AppError::Database)?;
        res.check().map_err(AppError::Database)?;

        Ok(())
    }

    /// Validates that an embedding vector matches the configured HNSW dimension.
    #[allow(clippy::result_large_err)]
    pub fn validate_dimension(embedding: &[f32], expected: usize) -> Result<(), AppError> {
        if embedding.len() != expected {
            return Err(AppError::Validation(format!(
                "embedding dimension mismatch: got {}, expected {expected}",
                embedding.len()
            )));
        }
        Ok(())
    }

    /// Create a new text chunk embedding.
    ///
    /// The embedding record id equals `chunk_id` so each chunk has at most one embedding row.
    /// `chunk_id` is the **key** part of the text_chunk id (e.g. the UUID), not "text_chunk:uuid".
    #[must_use]
    pub fn new(chunk_id: &str, source_id: String, embedding: Vec<f32>, user_id: String) -> Self {
        let now = Utc::now();

        Self {
            id: chunk_id.to_owned(),
            created_at: now,
            updated_at: now,
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
    /// This uses the denormalized `source_id` on the embedding table.
    pub async fn delete_by_source_id(
        source_id: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE FROM {} WHERE source_id = $source_id",
            Self::table_name()
        );

        db.client
            .query(query)
            .bind(("source_id", source_id.to_owned()))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use anyhow::{self, Context};

    use super::*;
    use crate::storage::db::SurrealDbClient;
    use crate::test_utils::{prepare_text_chunk_test_db, setup_test_db};
    use surrealdb::Value as SurrealValue;

    async fn create_text_chunk_with_id(
        db: &SurrealDbClient,
        key: &str,
        source_id: &str,
        user_id: &str,
    ) -> anyhow::Result<RecordId> {
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
            .with_context(|| "Failed to create text_chunk".to_string())?;

        Ok(RecordId::from_table_key(TextChunk::table_name(), key))
    }

    async fn get_idx_sql(db: &SurrealDbClient) -> anyhow::Result<String> {
        let mut info_res = db
            .client
            .query("INFO FOR TABLE text_chunk_embedding;")
            .await
            .with_context(|| "info query failed".to_string())?;
        let info: SurrealValue = info_res
            .take(0)
            .with_context(|| "failed to take info result".to_string())?;
        let info_json: serde_json::Value = serde_json::to_value(info)
            .with_context(|| "failed to convert info to json".to_string())?;
        let idx_sql = info_json
            .get("Object")
            .and_then(|v| v.get("indexes"))
            .and_then(|v| v.get("Object"))
            .and_then(|v| v.get("idx_embedding_text_chunk_embedding"))
            .and_then(|v| v.get("Strand"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(idx_sql)
    }

    #[test]
    fn new_uses_chunk_id_as_record_id() {
        let emb = TextChunkEmbedding::new(
            "chunk-abc",
            "source-1".to_owned(),
            vec![0.1, 0.2],
            "user-1".to_owned(),
        );
        assert_eq!(emb.id, "chunk-abc");
    }

    #[test]
    fn validate_dimension_rejects_mismatch() {
        let err = TextChunkEmbedding::validate_dimension(&[0.1, 0.2, 0.3], 2)
            .expect_err("expected dimension mismatch");
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn test_create_and_get_by_chunk_id() -> anyhow::Result<()> {
        let db = prepare_text_chunk_test_db(3).await?;

        let user_id = "user_a";
        let chunk_key = "chunk-123";
        let source_id = "source-1";

        let chunk_rid = create_text_chunk_with_id(&db, chunk_key, source_id, user_id).await?;

        let embedding_vec = vec![0.1_f32, 0.2, 0.3];
        let emb = TextChunkEmbedding::new(
            chunk_key,
            source_id.to_string(),
            embedding_vec.clone(),
            user_id.to_string(),
        );

        db.upsert_item(emb)
            .await
            .with_context(|| "Failed to store embedding".to_string())?;

        let fetched = TextChunkEmbedding::get_by_chunk_id(&chunk_rid, &db)
            .await
            .with_context(|| "Failed to get embedding by chunk_id".to_string())?
            .with_context(|| "Expected an embedding to be found".to_string())?;

        assert_eq!(fetched.id, chunk_key);
        assert_eq!(fetched.user_id, user_id);
        assert_eq!(fetched.chunk_id, chunk_rid);
        assert_eq!(fetched.embedding, embedding_vec);
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_chunk_id() -> anyhow::Result<()> {
        let db = prepare_text_chunk_test_db(3).await?;

        let user_id = "user_b";
        let chunk_key = "chunk-delete";
        let source_id = "source-del";

        let chunk_rid = create_text_chunk_with_id(&db, chunk_key, source_id, user_id).await?;

        let emb = TextChunkEmbedding::new(
            chunk_key,
            source_id.to_string(),
            vec![0.4_f32, 0.5, 0.6],
            user_id.to_string(),
        );

        db.upsert_item(emb)
            .await
            .with_context(|| "Failed to store embedding".to_string())?;

        let existing = TextChunkEmbedding::get_by_chunk_id(&chunk_rid, &db)
            .await
            .with_context(|| "Failed to get embedding before delete".to_string())?;
        assert!(existing.is_some(), "Embedding should exist before delete");

        TextChunkEmbedding::delete_by_chunk_id(&chunk_rid, &db)
            .await
            .with_context(|| "Failed to delete by chunk_id".to_string())?;

        let after = TextChunkEmbedding::get_by_chunk_id(&chunk_rid, &db)
            .await
            .with_context(|| "Failed to get embedding after delete".to_string())?;
        assert!(after.is_none(), "Embedding should have been deleted");
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_source_id() -> anyhow::Result<()> {
        let db = prepare_text_chunk_test_db(1).await?;

        let user_id = "user_c";
        let source_id = "shared-source";
        let other_source = "other-source";

        let chunk1_rid = create_text_chunk_with_id(&db, "chunk-s1", source_id, user_id).await?;
        let chunk2_rid = create_text_chunk_with_id(&db, "chunk-s2", source_id, user_id).await?;
        let chunk_other_rid =
            create_text_chunk_with_id(&db, "chunk-other", other_source, user_id).await?;

        for (key, src, vec) in [
            ("chunk-s1", source_id, vec![0.1]),
            ("chunk-s2", source_id, vec![0.2]),
            ("chunk-other", other_source, vec![0.3]),
        ] {
            let emb = TextChunkEmbedding::new(
                key,
                src.to_string(),
                vec,
                user_id.to_string(),
            );
            db.upsert_item(emb)
                .await
                .with_context(|| format!("store embedding for {key}"))?;
        }

        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk1_rid, &db)
            .await
            .with_context(|| "get chunk1".to_string())?
            .is_some());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk2_rid, &db)
            .await
            .with_context(|| "get chunk2".to_string())?
            .is_some());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk_other_rid, &db)
            .await
            .with_context(|| "get chunk_other".to_string())?
            .is_some());

        TextChunkEmbedding::delete_by_source_id(source_id, &db)
            .await
            .with_context(|| "Failed to delete by source_id".to_string())?;

        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk1_rid, &db)
            .await
            .with_context(|| "check chunk1".to_string())?
            .is_none());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk2_rid, &db)
            .await
            .with_context(|| "check chunk2".to_string())?
            .is_none());
        assert!(TextChunkEmbedding::get_by_chunk_id(&chunk_other_rid, &db)
            .await
            .with_context(|| "check chunk_other".to_string())?
            .is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_replaces_existing_embedding_row() -> anyhow::Result<()> {
        let db = prepare_text_chunk_test_db(3).await?;

        let user_id = "user-upsert";
        let source_id = "source-upsert";
        let chunk_key = "chunk-upsert";

        create_text_chunk_with_id(&db, chunk_key, source_id, user_id).await?;

        let initial = TextChunkEmbedding::new(
            chunk_key,
            source_id.to_owned(),
            vec![1.0_f32, 0.0, 0.0],
            user_id.to_owned(),
        );
        db.upsert_item(initial)
            .await
            .with_context(|| "initial upsert".to_string())?;

        let replacement = TextChunkEmbedding::new(
            chunk_key,
            source_id.to_owned(),
            vec![0.0, 1.0, 0.0],
            user_id.to_owned(),
        );
        db.upsert_item(replacement)
            .await
            .with_context(|| "upsert replacement embedding".to_string())?;

        let chunk_rid = RecordId::from_table_key(TextChunk::table_name(), chunk_key);
        let rows: Vec<TextChunkEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE chunk_id = $chunk_id",
                TextChunkEmbedding::table_name()
            ))
            .bind(("chunk_id", chunk_rid))
            .await
            .with_context(|| "count embeddings".to_string())?
            .take(0)
            .with_context(|| "take embeddings".to_string())?;

        assert_eq!(rows.len(), 1);
        let row = rows.first().expect("expected one embedding row");
        assert_eq!(row.id, chunk_key);
        assert_eq!(row.embedding, vec![0.0, 1.0, 0.0]);

        Ok(())
    }

    #[tokio::test]
    async fn test_redefine_hnsw_index_updates_dimension() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        TextChunkEmbedding::redefine_hnsw_index(&db, 8)
            .await
            .with_context(|| "failed to redefine index".to_string())?;

        let idx_sql = get_idx_sql(&db).await?;

        assert!(
            idx_sql.contains("DIMENSION 8"),
            "expected index definition to contain new dimension, got: {idx_sql}"
        );
        assert!(
            idx_sql.contains("DIST COSINE"),
            "expected index definition to use cosine distance, got: {idx_sql}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_redefine_hnsw_index_is_idempotent() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        TextChunkEmbedding::redefine_hnsw_index(&db, 4)
            .await
            .with_context(|| "first redefine failed".to_string())?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 4)
            .await
            .with_context(|| "second redefine failed".to_string())?;

        let idx_sql = get_idx_sql(&db).await?;

        assert!(
            idx_sql.contains("DIMENSION 4"),
            "expected index definition to retain dimension 4, got: {idx_sql}"
        );
        Ok(())
    }
}
