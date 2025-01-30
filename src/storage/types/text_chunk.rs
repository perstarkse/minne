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
