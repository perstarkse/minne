use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};
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
