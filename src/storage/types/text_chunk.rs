use crate::stored_object;
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
}
