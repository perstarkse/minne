#![allow(clippy::missing_docs_in_private_items, clippy::uninlined_format_args)]
use std::collections::HashMap;
use std::fmt::Write;

use crate::storage::types::text_chunk_embedding::TextChunkEmbedding;
use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};
use async_openai::{config::OpenAIConfig, Client};
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};

use tracing::{error, info};
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
        let query = format!(
            "DELETE {} WHERE source_id = '{}'",
            Self::table_name(),
            source_id
        );
        db_client.query(query).await?;

        Ok(())
    }

    /// Atomically store a text chunk and its embedding.
    /// Writes the chunk to `text_chunk` and the embedding to `text_chunk_embedding`.
    pub async fn store_with_embedding(
        chunk: TextChunk,
        embedding: Vec<f32>,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let chunk_id = chunk.id.clone();
        let source_id = chunk.source_id.clone();
        let user_id = chunk.user_id.clone();

        let emb = TextChunkEmbedding::new(&chunk_id, source_id.clone(), embedding, user_id.clone());

        // Create both records in a single transaction so we don't orphan embeddings or chunks
        let response = db
            .client
            .query("BEGIN TRANSACTION;")
            .query(format!(
                "CREATE type::thing('{chunk_table}', $chunk_id) CONTENT $chunk;",
                chunk_table = Self::table_name(),
            ))
            .query(format!(
                "CREATE type::thing('{emb_table}', $emb_id) CONTENT $emb;",
                emb_table = TextChunkEmbedding::table_name(),
            ))
            .query("COMMIT TRANSACTION;")
            .bind(("chunk_id", chunk_id.clone()))
            .bind(("chunk", chunk))
            .bind(("emb_id", emb.id.clone()))
            .bind(("emb", emb))
            .await
            .map_err(AppError::Database)?;

        response.check().map_err(AppError::Database)?;

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
            chunk_id: TextChunk,
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
            .map_err(|e| AppError::InternalError(format!("Surreal query failed: {e}")))?;

        let rows: Vec<Row> = response.take::<Vec<Row>>(0).unwrap_or_default();

        Ok(rows
            .into_iter()
            .map(|r| TextChunkSearchResult {
                chunk: r.chunk_id,
                score: r.score,
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
            .map_err(|e| AppError::InternalError(format!("Surreal query failed: {e}")))?;

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

    /// Re-creates embeddings for all text chunks using a safe, atomic transaction.
    ///
    /// This is a costly operation that should be run in the background. It performs these steps:
    /// 1. **Fetches All Chunks**: Loads all existing text_chunk records into memory.
    /// 2. **Generates All Embeddings**: Creates new embeddings for every chunk. If any fails or
    ///    has the wrong dimension, the entire operation is aborted before any DB changes are made.
    /// 3. **Executes Atomic Transaction**: All data updates and the index recreation are
    ///    performed in a single, all-or-nothing database transaction.
    pub async fn update_all_embeddings(
        db: &SurrealDbClient,
        openai_client: &Client<OpenAIConfig>,
        new_model: &str,
        new_dimensions: u32,
    ) -> Result<(), AppError> {
        info!(
            "Starting re-embedding process for all text chunks. New dimensions: {}",
            new_dimensions
        );

        // Fetch all chunks first
        let all_chunks: Vec<TextChunk> = db.select(Self::table_name()).await?;
        let total_chunks = all_chunks.len();
        if total_chunks == 0 {
            info!("No text chunks to update. Just updating the idx");

            TextChunkEmbedding::redefine_hnsw_index(db, new_dimensions as usize).await?;

            return Ok(());
        }
        info!("Found {} chunks to process.", total_chunks);

        // Generate all new embeddings in memory
        let mut new_embeddings: HashMap<String, (Vec<f32>, String, String)> = HashMap::new();
        info!("Generating new embeddings for all chunks...");
        for chunk in &all_chunks {
            let retry_strategy = ExponentialBackoff::from_millis(100).map(jitter).take(3);

            let embedding = Retry::spawn(retry_strategy, || {
                crate::utils::embedding::generate_embedding_with_params(
                    openai_client,
                    &chunk.chunk,
                    new_model,
                    new_dimensions,
                )
            })
            .await?;

            // Safety check: ensure the generated embedding has the correct dimension.
            if embedding.len() != new_dimensions as usize {
                let err_msg = format!(
                    "CRITICAL: Generated embedding for chunk {} has incorrect dimension ({}). Expected {}. Aborting.",
                    chunk.id, embedding.len(), new_dimensions
                );
                error!("{}", err_msg);
                return Err(AppError::InternalError(err_msg));
            }
            new_embeddings.insert(
                chunk.id.clone(),
                (embedding, chunk.user_id.clone(), chunk.source_id.clone()),
            );
        }
        info!("Successfully generated all new embeddings.");

        // Perform DB updates in a single transaction against the embedding table
        info!("Applying embedding updates in a transaction...");
        let mut transaction_query = String::from("BEGIN TRANSACTION;");

        for (id, (embedding, user_id, source_id)) in new_embeddings {
            let embedding_str = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            // Use the chunk id as the embedding record id to keep a 1:1 mapping
            write!(
                &mut transaction_query,
                "UPSERT type::thing('text_chunk_embedding', '{id}') SET \
                    chunk_id = type::thing('text_chunk', '{id}'), \
                    source_id = '{source_id}', \
                    embedding = {embedding}, \
                    user_id = '{user_id}', \
                    created_at = IF created_at != NONE THEN created_at ELSE time::now() END, \
                    updated_at = time::now();",
                id = id,
                embedding = embedding_str,
                user_id = user_id,
                source_id = source_id
            )
            .map_err(|e| AppError::InternalError(e.to_string()))?;
        }

        write!(
            &mut transaction_query,
            "DEFINE INDEX OVERWRITE idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding FIELDS embedding HNSW DIMENSION {};",
            new_dimensions
        )
        .map_err(|e| AppError::InternalError(e.to_string()))?;

        transaction_query.push_str("COMMIT TRANSACTION;");

        db.query(transaction_query).await?;

        info!("Re-embedding process for text chunks completed successfully.");
        Ok(())
    }

    /// Re-creates embeddings for all text chunks using an `EmbeddingProvider`.
    ///
    /// This variant uses the application's configured embedding provider (FastEmbed, OpenAI, etc.)
    /// instead of directly calling OpenAI. Used during startup when embedding configuration changes.
    pub async fn update_all_embeddings_with_provider(
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

        // Generate all new embeddings in memory
        let mut new_embeddings: HashMap<String, (Vec<f32>, String, String)> = HashMap::new();
        info!("Generating new embeddings for all chunks...");

        for (i, chunk) in all_chunks.iter().enumerate() {
            if i > 0 && i % 100 == 0 {
                info!(progress = i, total = total_chunks, "Re-embedding progress");
            }

            let embedding = provider
                .embed(&chunk.chunk)
                .await
                .map_err(|e| AppError::InternalError(format!("Embedding failed: {e}")))?;

            // Safety check: ensure the generated embedding has the correct dimension.
            if embedding.len() != new_dimensions {
                let err_msg = format!(
                    "CRITICAL: Generated embedding for chunk {} has incorrect dimension ({}). Expected {}. Aborting.",
                    chunk.id, embedding.len(), new_dimensions
                );
                error!("{}", err_msg);
                return Err(AppError::InternalError(err_msg));
            }
            new_embeddings.insert(
                chunk.id.clone(),
                (embedding, chunk.user_id.clone(), chunk.source_id.clone()),
            );
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
            let embedding_str = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            write!(
                &mut transaction_query,
                "CREATE type::thing('text_chunk_embedding', '{id}') SET \
                    chunk_id = type::thing('text_chunk', '{id}'), \
                    source_id = '{source_id}', \
                    embedding = {embedding}, \
                    user_id = '{user_id}', \
                    created_at = time::now(), \
                    updated_at = time::now();",
                id = id,
                embedding = embedding_str,
                user_id = user_id,
                source_id = source_id
            )
            .map_err(|e| AppError::InternalError(e.to_string()))?;
        }

        write!(
            &mut transaction_query,
            "DEFINE INDEX OVERWRITE idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding FIELDS embedding HNSW DIMENSION {};",
            new_dimensions
        )
        .map_err(|e| AppError::InternalError(e.to_string()))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::indexes::{ensure_runtime_indexes, rebuild_indexes};
    use crate::storage::types::text_chunk_embedding::TextChunkEmbedding;
    use surrealdb::RecordId;
    use uuid::Uuid;

    async fn ensure_chunk_fts_index(db: &SurrealDbClient) {
        let snowball_sql = r#"
            DEFINE ANALYZER IF NOT EXISTS app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii, snowball(english);
            DEFINE INDEX IF NOT EXISTS text_chunk_fts_chunk_idx ON TABLE text_chunk FIELDS chunk SEARCH ANALYZER app_en_fts_analyzer BM25;
        "#;

        if let Err(err) = db.client.query(snowball_sql).await {
            // Fall back to ascii-only analyzer when snowball is unavailable in the build.
            let fallback_sql = r#"
                DEFINE ANALYZER OVERWRITE app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii;
                DEFINE INDEX IF NOT EXISTS text_chunk_fts_chunk_idx ON TABLE text_chunk FIELDS chunk SEARCH ANALYZER app_en_fts_analyzer BM25;
            "#;

            db.client
                .query(fallback_sql)
                .await
                .unwrap_or_else(|_| panic!("define chunk fts index fallback: {err}"));
        }
    }

    #[tokio::test]
    async fn test_text_chunk_creation() {
        let source_id = "source123".to_string();
        let chunk = "This is a text chunk for testing embeddings".to_string();
        let user_id = "user123".to_string();

        let text_chunk = TextChunk::new(source_id.clone(), chunk.clone(), user_id.clone());

        assert_eq!(text_chunk.source_id, source_id);
        assert_eq!(text_chunk.chunk, chunk);
        assert_eq!(text_chunk.user_id, user_id);
        assert!(!text_chunk.id.is_empty());
    }

    #[tokio::test]
    async fn test_delete_by_source_id() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");

        let source_id = "source123".to_string();
        let user_id = "user123".to_string();
        TextChunkEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .expect("redefine index");

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
            .expect("store chunk1");
        TextChunk::store_with_embedding(chunk2.clone(), vec![0.1, 0.2, 0.3, 0.4, 0.5], &db)
            .await
            .expect("store chunk2");
        TextChunk::store_with_embedding(
            different_chunk.clone(),
            vec![0.1, 0.2, 0.3, 0.4, 0.5],
            &db,
        )
        .await
        .expect("store different chunk");

        TextChunk::delete_by_source_id(&source_id, &db)
            .await
            .expect("Failed to delete chunks by source_id");

        let remaining: Vec<TextChunk> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE source_id = '{}'",
                TextChunk::table_name(),
                source_id
            ))
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(remaining.len(), 0);

        let different_remaining: Vec<TextChunk> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE source_id = '{}'",
                TextChunk::table_name(),
                "different_source"
            ))
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(different_remaining.len(), 1);
        assert_eq!(different_remaining[0].id, different_chunk.id);
    }

    #[tokio::test]
    async fn test_delete_by_nonexistent_source_id() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");
        TextChunkEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .expect("redefine index");

        let real_source_id = "real_source".to_string();
        let chunk = TextChunk::new(
            real_source_id.clone(),
            "Test chunk".to_string(),
            "user123".to_string(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3, 0.4, 0.5], &db)
            .await
            .expect("store chunk");

        TextChunk::delete_by_source_id("nonexistent_source", &db)
            .await
            .expect("Delete should succeed");

        let remaining: Vec<TextChunk> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE source_id = '{}'",
                TextChunk::table_name(),
                real_source_id
            ))
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(remaining.len(), 1);
    }

    #[tokio::test]
    async fn test_store_with_embedding_creates_both_records() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");

        let source_id = "store-src".to_string();
        let user_id = "user_store".to_string();
        let chunk = TextChunk::new(source_id.clone(), "chunk body".to_string(), user_id.clone());

        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("redefine index");

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .expect("store with embedding");

        let stored_chunk: Option<TextChunk> = db.get_item(&chunk.id).await.unwrap();
        assert!(stored_chunk.is_some());
        let stored_chunk = stored_chunk.unwrap();
        assert_eq!(stored_chunk.source_id, source_id);
        assert_eq!(stored_chunk.user_id, user_id);

        let rid = RecordId::from_table_key(TextChunk::table_name(), &chunk.id);
        let embedding = TextChunkEmbedding::get_by_chunk_id(&rid, &db)
            .await
            .expect("get embedding");
        assert!(embedding.is_some());
        let embedding = embedding.unwrap();
        assert_eq!(embedding.chunk_id, rid);
        assert_eq!(embedding.user_id, user_id);
        assert_eq!(embedding.source_id, source_id);
    }

    #[tokio::test]
    async fn test_store_with_embedding_with_runtime_indexes() {
        let namespace = "test_ns_runtime";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");

        // Ensure runtime indexes are built with the expected dimension.
        let embedding_dimension = 3usize;
        ensure_runtime_indexes(&db, embedding_dimension)
            .await
            .expect("ensure runtime indexes");

        let chunk = TextChunk::new(
            "runtime_src".to_string(),
            "runtime chunk body".to_string(),
            "runtime_user".to_string(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .expect("store with embedding");

        let stored_chunk: Option<TextChunk> = db.get_item(&chunk.id).await.unwrap();
        assert!(stored_chunk.is_some(), "chunk should be stored");

        let rid = RecordId::from_table_key(TextChunk::table_name(), &chunk.id);
        let embedding = TextChunkEmbedding::get_by_chunk_id(&rid, &db)
            .await
            .expect("get embedding");
        assert!(embedding.is_some(), "embedding should exist");
        assert_eq!(
            embedding.unwrap().embedding.len(),
            embedding_dimension,
            "embedding dimension should match runtime index"
        );
    }

    #[tokio::test]
    async fn test_vector_search_returns_empty_when_no_embeddings() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");

        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("redefine index");

        let results: Vec<TextChunkSearchResult> =
            TextChunk::vector_search(5, vec![0.1, 0.2, 0.3], &db, "user")
                .await
                .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_vector_search_single_result() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");

        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("redefine index");

        let source_id = "src".to_string();
        let user_id = "user".to_string();
        let chunk = TextChunk::new(
            source_id.clone(),
            "hello world".to_string(),
            user_id.clone(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .expect("store");

        let results: Vec<TextChunkSearchResult> =
            TextChunk::vector_search(3, vec![0.1, 0.2, 0.3], &db, &user_id)
                .await
                .unwrap();

        assert_eq!(results.len(), 1);
        let res = &results[0];
        assert_eq!(res.chunk.id, chunk.id);
        assert_eq!(res.chunk.source_id, source_id);
        assert_eq!(res.chunk.chunk, "hello world");
    }

    #[tokio::test]
    async fn test_vector_search_orders_by_similarity() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");

        TextChunkEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("redefine index");

        let user_id = "user".to_string();
        let chunk1 = TextChunk::new("s1".to_string(), "chunk one".to_string(), user_id.clone());
        let chunk2 = TextChunk::new("s2".to_string(), "chunk two".to_string(), user_id.clone());

        TextChunk::store_with_embedding(chunk1.clone(), vec![1.0, 0.0, 0.0], &db)
            .await
            .expect("store chunk1");
        TextChunk::store_with_embedding(chunk2.clone(), vec![0.0, 1.0, 0.0], &db)
            .await
            .expect("store chunk2");

        let results: Vec<TextChunkSearchResult> =
            TextChunk::vector_search(2, vec![0.0, 1.0, 0.0], &db, &user_id)
                .await
                .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].chunk.id, chunk2.id);
        assert_eq!(results[1].chunk.id, chunk1.id);
        assert!(results[0].score >= results[1].score);
    }

    #[tokio::test]
    async fn test_fts_search_returns_empty_when_no_chunks() {
        let namespace = "fts_chunk_ns_empty";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");
        ensure_chunk_fts_index(&db).await;
        rebuild_indexes(&db).await.expect("rebuild indexes");

        let results = TextChunk::fts_search(5, "hello", &db, "user")
            .await
            .expect("fts search");

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_fts_search_single_result() {
        let namespace = "fts_chunk_ns_single";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");
        ensure_chunk_fts_index(&db).await;

        let user_id = "fts_user";
        let chunk = TextChunk::new(
            "fts_src".to_string(),
            "rustaceans love rust".to_string(),
            user_id.to_string(),
        );
        db.store_item(chunk.clone()).await.expect("store chunk");
        rebuild_indexes(&db).await.expect("rebuild indexes");

        let results = TextChunk::fts_search(3, "rust", &db, user_id)
            .await
            .expect("fts search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk.id, chunk.id);
        assert!(results[0].score.is_finite(), "expected a finite FTS score");
    }

    #[tokio::test]
    async fn test_fts_search_orders_by_score_and_filters_user() {
        let namespace = "fts_chunk_ns_order";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.expect("migrations");
        ensure_chunk_fts_index(&db).await;

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
            .expect("store high score chunk");
        db.store_item(low_score_chunk.clone())
            .await
            .expect("store low score chunk");
        db.store_item(other_user_chunk)
            .await
            .expect("store other user chunk");
        rebuild_indexes(&db).await.expect("rebuild indexes");

        let results = TextChunk::fts_search(3, "apple", &db, user_id)
            .await
            .expect("fts search");

        assert_eq!(results.len(), 2);
        let ids: Vec<_> = results.iter().map(|r| r.chunk.id.as_str()).collect();
        assert!(
            ids.contains(&high_score_chunk.id.as_str())
                && ids.contains(&low_score_chunk.id.as_str()),
            "expected only the two chunks for the same user"
        );
        assert!(
            results[0].score >= results[1].score,
            "expected results ordered by descending score"
        );
    }
}
