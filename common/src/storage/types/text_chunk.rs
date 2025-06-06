use std::collections::HashMap;

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
    embedding: Vec<f32>,
    user_id: String
});

impl TextChunk {
    pub fn new(source_id: String, chunk: String, embedding: Vec<f32>, user_id: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id,
            chunk,
            embedding,
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
            info!("No text chunks to update. Skipping.");
            return Ok(());
        }
        info!("Found {} chunks to process.", total_chunks);

        // Generate all new embeddings in memory
        let mut new_embeddings: HashMap<String, Vec<f32>> = HashMap::new();
        info!("Generating new embeddings for all chunks...");
        for chunk in all_chunks.iter() {
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
            new_embeddings.insert(chunk.id.clone(), embedding);
        }
        info!("Successfully generated all new embeddings.");

        // Perform DB updates in a single transaction
        info!("Applying schema and data changes in a transaction...");
        let mut transaction_query = String::from("BEGIN TRANSACTION;");

        // Add all update statements
        for (id, embedding) in new_embeddings {
            let embedding_str = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            transaction_query.push_str(&format!(
                "UPDATE type::thing('text_chunk', '{}') SET embedding = {}, updated_at = time::now();",
                id, embedding_str
            ));
        }

        // Re-create the index inside the same transaction
        transaction_query.push_str("REMOVE INDEX idx_embedding_chunks ON TABLE text_chunk;");
        transaction_query.push_str(&format!(
            "DEFINE INDEX idx_embedding_chunks ON TABLE text_chunk FIELDS embedding HNSW DIMENSION {};",
            new_dimensions
        ));

        transaction_query.push_str("COMMIT TRANSACTION;");

        // Execute the entire atomic operation
        db.query(transaction_query).await?;

        info!("Re-embedding process for text chunks completed successfully.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_text_chunk_creation() {
        // Test basic object creation
        let source_id = "source123".to_string();
        let chunk = "This is a text chunk for testing embeddings".to_string();
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let user_id = "user123".to_string();

        let text_chunk = TextChunk::new(
            source_id.clone(),
            chunk.clone(),
            embedding.clone(),
            user_id.clone(),
        );

        // Check that the fields are set correctly
        assert_eq!(text_chunk.source_id, source_id);
        assert_eq!(text_chunk.chunk, chunk);
        assert_eq!(text_chunk.embedding, embedding);
        assert_eq!(text_chunk.user_id, user_id);
        assert!(!text_chunk.id.is_empty());
    }

    #[tokio::test]
    async fn test_delete_by_source_id() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create test data
        let source_id = "source123".to_string();
        let chunk1 = "First chunk from the same source".to_string();
        let chunk2 = "Second chunk from the same source".to_string();
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let user_id = "user123".to_string();

        // Create two chunks with the same source_id
        let text_chunk1 = TextChunk::new(
            source_id.clone(),
            chunk1,
            embedding.clone(),
            user_id.clone(),
        );

        let text_chunk2 = TextChunk::new(
            source_id.clone(),
            chunk2,
            embedding.clone(),
            user_id.clone(),
        );

        // Create a chunk with a different source_id
        let different_source_id = "different_source".to_string();
        let different_chunk = TextChunk::new(
            different_source_id.clone(),
            "Different source chunk".to_string(),
            embedding.clone(),
            user_id.clone(),
        );

        // Store the chunks
        db.store_item(text_chunk1)
            .await
            .expect("Failed to store text chunk 1");
        db.store_item(text_chunk2)
            .await
            .expect("Failed to store text chunk 2");
        db.store_item(different_chunk.clone())
            .await
            .expect("Failed to store different chunk");

        // Delete by source_id
        TextChunk::delete_by_source_id(&source_id, &db)
            .await
            .expect("Failed to delete chunks by source_id");

        // Verify all chunks with the original source_id are deleted
        let query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            TextChunk::table_name(),
            source_id
        );
        let remaining: Vec<TextChunk> = db
            .client
            .query(query)
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(
            remaining.len(),
            0,
            "All chunks with the source_id should be deleted"
        );

        // Verify the different source_id chunk still exists
        let different_query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            TextChunk::table_name(),
            different_source_id
        );
        let different_remaining: Vec<TextChunk> = db
            .client
            .query(different_query)
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(
            different_remaining.len(),
            1,
            "Chunk with different source_id should still exist"
        );
        assert_eq!(different_remaining[0].id, different_chunk.id);
    }

    #[tokio::test]
    async fn test_delete_by_nonexistent_source_id() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a chunk with a real source_id
        let real_source_id = "real_source".to_string();
        let chunk = "Test chunk".to_string();
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let user_id = "user123".to_string();

        let text_chunk = TextChunk::new(real_source_id.clone(), chunk, embedding, user_id);

        // Store the chunk
        db.store_item(text_chunk)
            .await
            .expect("Failed to store text chunk");

        // Delete using nonexistent source_id
        let nonexistent_source_id = "nonexistent_source";
        TextChunk::delete_by_source_id(nonexistent_source_id, &db)
            .await
            .expect("Delete operation with nonexistent source_id should not fail");

        // Verify the real chunk still exists
        let query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            TextChunk::table_name(),
            real_source_id
        );
        let remaining: Vec<TextChunk> = db
            .client
            .query(query)
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(
            remaining.len(),
            1,
            "Chunk with real source_id should still exist"
        );
    }
}
