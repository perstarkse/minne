#![allow(clippy::missing_docs_in_private_items)]
use std::collections::HashMap;
use std::fmt::Write;

use crate::storage::indexes::hnsw_index_overwrite_sql;
use crate::storage::types::system_settings::SystemSettings;
use crate::storage::types::text_chunk_embedding::TextChunkEmbedding;
use crate::utils::embedding::RE_EMBED_BATCH_SIZE;
use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use tracing::{error, info, warn};
use uuid::Uuid;

stored_object!(TextChunk, "text_chunk", {
    source_id: String,
    chunk: String,
    user_id: String
});

/// Search result including hydrated chunk.
#[allow(clippy::module_name_repetitions)]
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TextChunkSearchResult {
    pub chunk: TextChunk,
    pub score: f32,
}

impl TextChunk {
    #[must_use]
    pub fn new(source_id: String, chunk: String, user_id: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id,
            chunk,
            user_id,
        }
    }

    pub async fn delete_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        db_client
            .client
            .query("BEGIN TRANSACTION;")
            .query(format!(
                "DELETE FROM {} WHERE source_id = $source_id;",
                TextChunkEmbedding::table_name()
            ))
            .query("DELETE FROM type::table($table) WHERE source_id = $source_id;")
            .query("COMMIT TRANSACTION;")
            .bind(("source_id", source_id.to_owned()))
            .bind(("table", Self::table_name()))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        Ok(())
    }

    /// Atomically store a text chunk and its embedding.
    /// Writes the chunk to `text_chunk` and the embedding to `text_chunk_embedding`.
    pub async fn store_with_embedding(
        chunk: TextChunk,
        embedding: Vec<f32>,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let settings = SystemSettings::get_current(db).await?;
        TextChunkEmbedding::validate_dimension(&embedding, settings.embedding_dimensions as usize)?;

        let chunk_id = chunk.id.clone();
        let emb = TextChunkEmbedding::new(
            &chunk_id,
            chunk.source_id.clone(),
            embedding,
            chunk.user_id.clone(),
        );

        let query = format!(
            "
            BEGIN TRANSACTION;
              CREATE type::thing('{chunk_table}', $chunk_id) CONTENT $chunk;
              UPSERT type::thing('{emb_table}', $chunk_id) CONTENT $emb;
            COMMIT TRANSACTION;
            ",
            chunk_table = Self::table_name(),
            emb_table = TextChunkEmbedding::table_name(),
        );

        db.client
            .query(query)
            .bind(("chunk_id", chunk_id))
            .bind(("chunk", chunk))
            .bind(("emb", emb))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        Ok(())
    }

    /// Vector search over text chunks using the embedding table, fetching full chunk rows and embeddings.
    pub async fn vector_search(
        take: usize,
        query_embedding: Vec<f32>,
        db: &SurrealDbClient,
        user_id: &str,
    ) -> Result<Vec<TextChunkSearchResult>, AppError> {
        #[allow(clippy::missing_docs_in_private_items)]
        #[derive(Deserialize)]
        struct Row {
            chunk_id: Option<TextChunk>,
            score: f32,
        }

        let sql = format!(
            r#"
            SELECT
                chunk_id,
                embedding,
                vector::similarity::cosine(embedding, $embedding) AS score
            FROM {emb_table}
            WHERE user_id = $user_id
              AND embedding <|{take},100|> $embedding
            ORDER BY score DESC
            LIMIT {take}
            FETCH chunk_id;
            "#,
            emb_table = TextChunkEmbedding::table_name(),
            take = take
        );

        let mut response = db
            .query(&sql)
            .bind(("embedding", query_embedding))
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(AppError::Database)?;

        response = response.check().map_err(AppError::Database)?;

        let rows: Vec<Row> = response.take::<Vec<Row>>(0).map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                r.chunk_id.map(|chunk| TextChunkSearchResult {
                    chunk,
                    score: r.score,
                }).or_else(|| {
                    warn!("vector search hit orphaned text_chunk_embedding row with missing chunk");
                    None
                })
            })
            .collect())
    }

    /// Full-text search over text chunks using the BM25 FTS index.
    pub async fn fts_search(
        take: usize,
        terms: &str,
        db: &SurrealDbClient,
        user_id: &str,
    ) -> Result<Vec<TextChunkSearchResult>, AppError> {
        #[derive(Deserialize)]
        struct Row {
            #[serde(deserialize_with = "deserialize_flexible_id")]
            id: String,
            #[serde(deserialize_with = "deserialize_datetime")]
            created_at: DateTime<Utc>,
            #[serde(deserialize_with = "deserialize_datetime")]
            updated_at: DateTime<Utc>,
            source_id: String,
            chunk: String,
            user_id: String,
            score: f32,
        }

        let limit = i64::try_from(take).unwrap_or(i64::MAX);

        let sql = format!(
            r#"
            SELECT
                id,
                created_at,
                updated_at,
                source_id,
                chunk,
                user_id,
                IF search::score(0) != NONE THEN search::score(0) ELSE 0 END AS score
            FROM {chunk_table}
            WHERE chunk @0@ $terms
              AND user_id = $user_id
            ORDER BY score DESC
            LIMIT $limit;
            "#,
            chunk_table = Self::table_name(),
        );

        let mut response = db
            .query(&sql)
            .bind(("terms", terms.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("limit", limit))
            .await
            .map_err(AppError::Database)?;

        response = response.check().map_err(AppError::Database)?;

        let rows: Vec<Row> = response.take::<Vec<Row>>(0).map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let chunk = TextChunk {
                    id: r.id,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                    source_id: r.source_id,
                    chunk: r.chunk,
                    user_id: r.user_id,
                };

                TextChunkSearchResult {
                    chunk,
                    score: r.score,
                }
            })
            .collect())
    }

    /// Re-creates embeddings for all text chunks using an `EmbeddingProvider`.
    ///
    /// This is a costly operation that should be run in the background. It performs these steps:
    /// 1. **Fetches All Chunks**: Loads all existing text_chunk records into memory.
    /// 2. **Generates All Embeddings**: Creates new embeddings for every chunk. If any fails or
    ///    has the wrong dimension, the entire operation is aborted before any DB changes are made.
    /// 3. **Executes Atomic Transaction**: All data updates and the index recreation are
    ///    performed in a single, all-or-nothing database transaction.
    #[allow(clippy::too_many_lines)]
    pub async fn update_all_embeddings(
        db: &SurrealDbClient,
        provider: &crate::utils::embedding::EmbeddingProvider,
    ) -> Result<(), AppError> {
        let new_dimensions = provider.dimension();
        info!(
            dimensions = new_dimensions,
            backend = provider.backend_label(),
            "Starting re-embedding process for all text chunks"
        );

        // Fetch all chunks first
        let all_chunks: Vec<TextChunk> = db.select(Self::table_name()).await?;
        let total_chunks = all_chunks.len();
        if total_chunks == 0 {
            info!("No text chunks to update. Just updating the index.");
            TextChunkEmbedding::redefine_hnsw_index(db, new_dimensions).await?;
            return Ok(());
        }
        info!(chunks = total_chunks, "Found chunks to process");

        // Generate all new embeddings in memory, batching to amortise lock/dispatch overhead
        // while keeping memory bounded and preserving progress logging.
        let mut new_embeddings: HashMap<String, (Vec<f32>, String, String)> =
            HashMap::with_capacity(total_chunks);
        info!("Generating new embeddings for all chunks...");

        let mut processed = 0usize;
        for batch in all_chunks.chunks(RE_EMBED_BATCH_SIZE) {
            let inputs: Vec<String> = batch.iter().map(|chunk| chunk.chunk.clone()).collect();
            let embeddings = provider.embed_batch(inputs).await?;
            if embeddings.len() != batch.len() {
                return Err(AppError::internal(format!(
                    "embedding batch returned {} vectors for {} chunks",
                    embeddings.len(),
                    batch.len()
                )));
            }

            for (chunk, embedding) in batch.iter().zip(embeddings) {
                // Safety check: ensure the generated embedding has the correct dimension.
                if embedding.len() != new_dimensions {
                    let err_msg = format!(
                        "CRITICAL: Generated embedding for chunk {} has incorrect dimension ({}). Expected {}. Aborting.",
                        chunk.id, embedding.len(), new_dimensions
                    );
                    error!("{err_msg}");
                    return Err(AppError::internal(err_msg));
                }
                new_embeddings.insert(
                    chunk.id.clone(),
                    (embedding, chunk.user_id.clone(), chunk.source_id.clone()),
                );
            }

            processed = processed.saturating_add(batch.len());
            info!(progress = processed, total = total_chunks, "Re-embedding progress");
        }
        info!("Successfully generated all new embeddings.");

        // Clear existing embeddings and index first to prevent SurrealDB panics and dimension conflicts.
        info!("Removing old index and clearing embeddings...");

        // Explicitly remove the index first. This prevents background HNSW maintenance from crashing
        // when we delete/replace data, dealing with a known SurrealDB panic.
        db.client
            .query(format!(
                "REMOVE INDEX idx_embedding_text_chunk_embedding ON TABLE {};",
                TextChunkEmbedding::table_name()
            ))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        db.client
            .query(format!("DELETE FROM {};", TextChunkEmbedding::table_name()))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        // Perform DB updates in a single transaction against the embedding table
        info!("Applying embedding updates in a transaction...");
        let mut transaction_query = String::from("BEGIN TRANSACTION;");

        for (id, (embedding, user_id, source_id)) in new_embeddings {
            let embedding = serde_json::to_string(&embedding)
                .map_err(|e| AppError::internal(format!("embedding serialization failed: {e}")))?;
            let id = surql_json_string(&id)?;
            let user_id = surql_json_string(&user_id)?;
            let source_id = surql_json_string(&source_id)?;
            write!(
                &mut transaction_query,
                "CREATE type::thing('{emb_table}', {id}) SET \
                    chunk_id = type::thing('{chunk_table}', {id}), \
                    source_id = {source_id}, \
                    embedding = {embedding}, \
                    user_id = {user_id}, \
                    created_at = time::now(), \
                    updated_at = time::now();",
                emb_table = TextChunkEmbedding::table_name(),
                chunk_table = Self::table_name(),
            )
            .map_err(AppError::internal)?;
        }

        write!(
            &mut transaction_query,
            "{}",
            hnsw_index_overwrite_sql(
                "idx_embedding_text_chunk_embedding",
                TextChunkEmbedding::table_name(),
                new_dimensions,
            )
        )
        .map_err(AppError::internal)?;

        transaction_query.push_str("COMMIT TRANSACTION;");

        db.client
            .query(transaction_query)
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        info!("Re-embedding process for text chunks completed successfully.");
        Ok(())
    }
}

#[allow(clippy::result_large_err)]
fn surql_json_string(value: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|e| AppError::internal(format!("string serialization failed: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use anyhow::{self, Context};

    use super::*;
    use crate::storage::indexes::{ensure_runtime, rebuild};
    use crate::storage::types::text_chunk_embedding::TextChunkEmbedding;
    use crate::test_utils::configure_embedding_dimension;
    use surrealdb::RecordId;
    use uuid::Uuid;

    async fn ensure_chunk_fts_index(db: &SurrealDbClient) -> anyhow::Result<()> {
        let snowball_sql = r#"
            DEFINE ANALYZER IF NOT EXISTS app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii, snowball(english);
            DEFINE INDEX IF NOT EXISTS text_chunk_fts_chunk_idx ON TABLE text_chunk FIELDS chunk SEARCH ANALYZER app_en_fts_analyzer BM25;
        "#;

        if let Err(err) = db.client.query(snowball_sql).await {
            let fallback_sql = r#"
                DEFINE ANALYZER OVERWRITE app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii;
                DEFINE INDEX IF NOT EXISTS text_chunk_fts_chunk_idx ON TABLE text_chunk FIELDS chunk SEARCH ANALYZER app_en_fts_analyzer BM25;
            "#;

            db.client
                .query(fallback_sql)
                .await
                .with_context(|| format!("define chunk fts index fallback: {err}"))?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_text_chunk_creation() -> anyhow::Result<()> {
        let source_id = "source123".to_string();
        let chunk = "This is a text chunk for testing embeddings".to_string();
        let user_id = "user123".to_string();

        let text_chunk = TextChunk::new(source_id.clone(), chunk.clone(), user_id.clone());

        assert_eq!(text_chunk.source_id, source_id);
        assert_eq!(text_chunk.chunk, chunk);
        assert_eq!(text_chunk.user_id, user_id);
        assert!(!text_chunk.id.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_source_id() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;

        let source_id = "source123".to_string();
        let user_id = "user123".to_string();
        configure_embedding_dimension(&db, 5).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .with_context(|| "redefine index".to_string())?;

        let chunk1 = TextChunk::new(
            source_id.clone(),
            "First chunk from the same source".to_string(),
            user_id.clone(),
        );
        let chunk2 = TextChunk::new(
            source_id.clone(),
            "Second chunk from the same source".to_string(),
            user_id.clone(),
        );
        let different_chunk = TextChunk::new(
            "different_source".to_string(),
            "Different source chunk".to_string(),
            user_id.clone(),
        );

        TextChunk::store_with_embedding(chunk1.clone(), vec![0.1, 0.2, 0.3, 0.4, 0.5], &db)
            .await
            .with_context(|| "store chunk1".to_string())?;
        TextChunk::store_with_embedding(chunk2.clone(), vec![0.1, 0.2, 0.3, 0.4, 0.5], &db)
            .await
            .with_context(|| "store chunk2".to_string())?;
        TextChunk::store_with_embedding(
            different_chunk.clone(),
            vec![0.1, 0.2, 0.3, 0.4, 0.5],
            &db,
        )
        .await
        .with_context(|| "store different chunk".to_string())?;

        TextChunk::delete_by_source_id(&source_id, &db)
            .await
            .with_context(|| "Failed to delete chunks by source_id".to_string())?;

        let remaining: Vec<TextChunk> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE source_id = '{source_id}'",
                TextChunk::table_name(),
            ))
            .await
            .with_context(|| "Query failed".to_string())?
            .take(0)
            .with_context(|| "Failed to get query results".to_string())?;
        assert_eq!(remaining.len(), 0);

        let different_remaining: Vec<TextChunk> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE source_id = 'different_source'",
                TextChunk::table_name(),
            ))
            .await
            .with_context(|| "Query failed".to_string())?
            .take(0)
            .with_context(|| "Failed to get query results".to_string())?;
        assert_eq!(different_remaining.len(), 1);
        assert_eq!(
            different_remaining.first().map(|r| &r.id),
            Some(&different_chunk.id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_nonexistent_source_id() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        configure_embedding_dimension(&db, 5).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .with_context(|| "redefine index".to_string())?;

        let real_source_id = "real_source".to_string();
        let chunk = TextChunk::new(
            real_source_id.clone(),
            "Test chunk".to_string(),
            "user123".to_string(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3, 0.4, 0.5], &db)
            .await
            .with_context(|| "store chunk".to_string())?;

        TextChunk::delete_by_source_id("nonexistent_source", &db)
            .await
            .with_context(|| "Delete should succeed".to_string())?;

        let remaining: Vec<TextChunk> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE source_id = '{real_source_id}'",
                TextChunk::table_name(),
            ))
            .await
            .with_context(|| "Query failed".to_string())?
            .take(0)
            .with_context(|| "Failed to get query results".to_string())?;
        assert_eq!(remaining.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_source_id_resists_query_injection() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");
        configure_embedding_dimension(&db, 5)
            .await
            .expect("configure dim");
        TextChunkEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .expect("redefine index");

        let chunk1 = TextChunk::new(
            "safe_source".to_string(),
            "Safe chunk".to_string(),
            "user123".to_string(),
        );
        let chunk2 = TextChunk::new(
            "other_source".to_string(),
            "Other chunk".to_string(),
            "user123".to_string(),
        );

        TextChunk::store_with_embedding(chunk1.clone(), vec![0.1, 0.2, 0.3, 0.4, 0.5], &db)
            .await
            .expect("store chunk1");
        TextChunk::store_with_embedding(chunk2.clone(), vec![0.5, 0.4, 0.3, 0.2, 0.1], &db)
            .await
            .expect("store chunk2");

        let malicious_source = "safe_source' OR 1=1 --";
        TextChunk::delete_by_source_id(malicious_source, &db)
            .await
            .expect("delete call should succeed");

        let remaining: Vec<TextChunk> = db
            .client
            .query("SELECT * FROM type::table($table)")
            .bind(("table", TextChunk::table_name()))
            .await
            .expect("query failed")
            .take(0)
            .expect("take failed");

        assert_eq!(
            remaining.len(),
            2,
            "malicious input must not delete unrelated rows"
        );
    }

    #[tokio::test]
    async fn test_store_with_embedding_creates_both_records() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;

        let source_id = "store-src".to_string();
        let user_id = "user_store".to_string();
        let chunk = TextChunk::new(source_id.clone(), "chunk body".to_string(), user_id.clone());

        configure_embedding_dimension(&db, 3).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "redefine index".to_string())?;

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .with_context(|| "store with embedding".to_string())?;

        let stored_chunk: Option<TextChunk> = db
            .get_item(&chunk.id)
            .await
            .with_context(|| "get_item".to_string())?;
        let stored_chunk = stored_chunk.with_context(|| "expected stored chunk".to_string())?;
        assert_eq!(stored_chunk.source_id, source_id);
        assert_eq!(stored_chunk.user_id, user_id);

        let rid = RecordId::from_table_key(TextChunk::table_name(), &chunk.id);
        let embedding = TextChunkEmbedding::get_by_chunk_id(&rid, &db)
            .await
            .with_context(|| "get embedding".to_string())?
            .with_context(|| "expected embedding".to_string())?;
        assert_eq!(embedding.chunk_id, rid);
        assert_eq!(embedding.id, chunk.id);
        assert_eq!(embedding.user_id, user_id);
        assert_eq!(embedding.source_id, source_id);
        Ok(())
    }

    #[tokio::test]
    async fn test_store_with_embedding_with_runtime_indexes() -> anyhow::Result<()> {
        let namespace = "test_ns_runtime";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;

        let embedding_dimension = 3usize;
        configure_embedding_dimension(
            &db,
            u32::try_from(embedding_dimension).expect("test embedding dimension fits in u32"),
        )
        .await?;
        ensure_runtime(&db, embedding_dimension)
            .await
            .with_context(|| "ensure runtime indexes".to_string())?;

        let chunk = TextChunk::new(
            "runtime_src".to_string(),
            "runtime chunk body".to_string(),
            "runtime_user".to_string(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .with_context(|| "store with embedding".to_string())?;

        let stored_chunk: Option<TextChunk> = db
            .get_item(&chunk.id)
            .await
            .with_context(|| "get_item".to_string())?;
        let stored_chunk = stored_chunk.with_context(|| "chunk should be stored".to_string())?;
        assert!(stored_chunk.id == chunk.id, "chunk should be stored");

        let rid = RecordId::from_table_key(TextChunk::table_name(), &chunk.id);
        let embedding = TextChunkEmbedding::get_by_chunk_id(&rid, &db)
            .await
            .with_context(|| "get embedding".to_string())?
            .with_context(|| "embedding should exist".to_string())?;
        assert_eq!(
            embedding.embedding.len(),
            embedding_dimension,
            "embedding dimension should match runtime index"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_vector_search_returns_empty_when_no_embeddings() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;

        configure_embedding_dimension(&db, 3).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "redefine index".to_string())?;

        let results: Vec<TextChunkSearchResult> =
            TextChunk::vector_search(5, vec![0.1, 0.2, 0.3], &db, "user")
                .await
                .with_context(|| "vector_search".to_string())?;
        assert!(results.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_vector_search_single_result() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;

        configure_embedding_dimension(&db, 3).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "redefine index".to_string())?;

        let source_id = "src".to_string();
        let user_id = "user".to_string();
        let chunk = TextChunk::new(
            source_id.clone(),
            "hello world".to_string(),
            user_id.clone(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .with_context(|| "store".to_string())?;

        let results: Vec<TextChunkSearchResult> =
            TextChunk::vector_search(3, vec![0.1, 0.2, 0.3], &db, &user_id)
                .await
                .with_context(|| "vector_search".to_string())?;

        assert_eq!(results.len(), 1);
        let res = results.first().context("expected first result")?;
        assert_eq!(res.chunk.id, chunk.id);
        assert_eq!(res.chunk.source_id, source_id);
        assert_eq!(res.chunk.chunk, "hello world");
        Ok(())
    }

    #[tokio::test]
    async fn test_vector_search_orders_by_similarity() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;

        configure_embedding_dimension(&db, 3).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "redefine index".to_string())?;

        let user_id = "user".to_string();
        let chunk1 = TextChunk::new("s1".to_string(), "chunk one".to_string(), user_id.clone());
        let chunk2 = TextChunk::new("s2".to_string(), "chunk two".to_string(), user_id.clone());

        TextChunk::store_with_embedding(chunk1.clone(), vec![1.0, 0.0, 0.0], &db)
            .await
            .with_context(|| "store chunk1".to_string())?;
        TextChunk::store_with_embedding(chunk2.clone(), vec![0.0, 1.0, 0.0], &db)
            .await
            .with_context(|| "store chunk2".to_string())?;

        let results: Vec<TextChunkSearchResult> =
            TextChunk::vector_search(2, vec![0.0, 1.0, 0.0], &db, &user_id)
                .await
                .with_context(|| "vector_search".to_string())?;

        assert_eq!(results.len(), 2);
        assert_eq!(results.first().map(|r| &r.chunk.id), Some(&chunk2.id));
        assert_eq!(results.get(1).map(|r| &r.chunk.id), Some(&chunk1.id));
        let r0 = results.first().context("expected first result")?;
        let r1 = results.get(1).context("expected second result")?;
        assert!(r0.score >= r1.score);
        Ok(())
    }

    #[tokio::test]
    async fn test_fts_search_returns_empty_when_no_chunks() -> anyhow::Result<()> {
        let namespace = "fts_chunk_ns_empty";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        ensure_chunk_fts_index(&db).await?;
        rebuild(&db)
            .await
            .with_context(|| "rebuild indexes".to_string())?;

        let results = TextChunk::fts_search(5, "hello", &db, "user")
            .await
            .with_context(|| "fts search".to_string())?;

        assert!(results.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_fts_search_single_result() -> anyhow::Result<()> {
        let namespace = "fts_chunk_ns_single";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        ensure_chunk_fts_index(&db).await?;

        let user_id = "fts_user";
        let chunk = TextChunk::new(
            "fts_src".to_string(),
            "rustaceans love rust".to_string(),
            user_id.to_string(),
        );
        db.store_item(chunk.clone())
            .await
            .with_context(|| "store chunk".to_string())?;
        rebuild(&db)
            .await
            .with_context(|| "rebuild indexes".to_string())?;

        let results = TextChunk::fts_search(3, "rust", &db, user_id)
            .await
            .with_context(|| "fts search".to_string())?;

        assert_eq!(results.len(), 1);
        let r0 = results.first().context("expected first result")?;
        assert_eq!(r0.chunk.id, chunk.id);
        assert!(r0.score.is_finite(), "expected a finite FTS score");
        Ok(())
    }

    #[tokio::test]
    async fn test_fts_search_orders_by_score_and_filters_user() -> anyhow::Result<()> {
        let namespace = "fts_chunk_ns_order";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        ensure_chunk_fts_index(&db).await?;

        let user_id = "fts_user_order";
        let high_score_chunk = TextChunk::new(
            "src1".to_string(),
            "apple apple apple pie recipe".to_string(),
            user_id.to_string(),
        );
        let low_score_chunk = TextChunk::new(
            "src2".to_string(),
            "apple tart".to_string(),
            user_id.to_string(),
        );
        let other_user_chunk = TextChunk::new(
            "src3".to_string(),
            "apple orchard guide".to_string(),
            "other_user".to_string(),
        );

        db.store_item(high_score_chunk.clone())
            .await
            .with_context(|| "store high score chunk".to_string())?;
        db.store_item(low_score_chunk.clone())
            .await
            .with_context(|| "store low score chunk".to_string())?;
        db.store_item(other_user_chunk)
            .await
            .with_context(|| "store other user chunk".to_string())?;
        rebuild(&db)
            .await
            .with_context(|| "rebuild indexes".to_string())?;

        let results = TextChunk::fts_search(3, "apple", &db, user_id)
            .await
            .with_context(|| "fts search".to_string())?;

        assert_eq!(results.len(), 2);
        let ids: Vec<_> = results.iter().map(|r| r.chunk.id.as_str()).collect();
        assert!(
            ids.contains(&high_score_chunk.id.as_str())
                && ids.contains(&low_score_chunk.id.as_str()),
            "expected only the two chunks for the same user"
        );
        let r0 = results.first().context("expected first result")?;
        let r1 = results.get(1).context("expected second result")?;
        assert!(
            r0.score >= r1.score,
            "expected results ordered by descending score"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_store_with_embedding_rejects_wrong_dimension() -> anyhow::Result<()> {
        let namespace = "test_ns_dim";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        configure_embedding_dimension(&db, 3).await?;

        let chunk = TextChunk::new("src".to_string(), "body".to_string(), "user".to_string());

        let err = TextChunk::store_with_embedding(chunk, vec![0.1, 0.2], &db)
            .await
            .expect_err("expected dimension validation failure");
        assert!(matches!(err, AppError::Validation(_)));

        Ok(())
    }

    #[tokio::test]
    async fn test_vector_search_with_orphaned_embedding() -> anyhow::Result<()> {
        let namespace = "test_ns_orphan_chunk";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        configure_embedding_dimension(&db, 3).await?;
        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "redefine index".to_string())?;

        let user_id = "user".to_string();
        let chunk = TextChunk::new(
            "src".to_string(),
            "orphan chunk".to_string(),
            user_id.clone(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .with_context(|| "store chunk with embedding".to_string())?;

        db.client
            .query(format!(
                "DELETE type::thing('{table}', $id);",
                table = TextChunk::table_name()
            ))
            .bind(("id", chunk.id.clone()))
            .await
            .with_context(|| "delete chunk".to_string())?;

        let results = TextChunk::vector_search(3, vec![0.1, 0.2, 0.3], &db, &user_id)
            .await
            .with_context(|| "search should succeed even with orphans".to_string())?;

        assert!(
            results.is_empty(),
            "should return empty result for orphan, got: {results:?}"
        );

        Ok(())
    }
}
