use crate::stored_object;
use uuid::Uuid;

stored_object!(TextChunk, "text_chunk", {
    source_id: String,
    chunk: String,
    embedding: Vec<f32>
});

impl TextChunk {
    pub fn new(source_id: String, chunk: String, embedding: Vec<f32>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            source_id,
            chunk,
            embedding,
        }
    }
}
