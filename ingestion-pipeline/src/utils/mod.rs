pub mod audio_transcription;
pub mod image_parsing;
pub mod llm_instructions;
pub mod pdf_ingestion;

use common::error::AppError;
use std::collections::HashMap;
use uuid::Uuid;

/// Intermediate struct to hold mapping between LLM keys and generated IDs.
#[derive(Clone)]
pub struct GraphMapper {
    pub key_to_id: HashMap<String, Uuid>,
}

impl Default for GraphMapper {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphMapper {
    pub fn new() -> Self {
        Self {
            key_to_id: HashMap::new(),
        }
    }
    /// Tries to get an ID by first parsing the key as a UUID,
    /// and if that fails, looking it up in the internal map.
    pub fn get_or_parse_id(&self, key: &str) -> Result<Uuid, AppError> {
        // First, try to parse the key as a UUID.
        if let Ok(parsed_uuid) = Uuid::parse_str(key) {
            return Ok(parsed_uuid);
        }

        // If parsing fails, look it up in the map.
        self.key_to_id.get(key).copied().ok_or_else(|| {
            AppError::GraphMapper(format!(
                "Key '{key}' is not a valid UUID and was not found in the map."
            ))
        })
    }

    /// Assigns a new UUID for a given key. (No changes needed here)
    pub fn assign_id(&mut self, key: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.key_to_id.insert(key.to_string(), id);
        id
    }

    /// Retrieves the UUID for a given key, returning a Result for consistency.
    pub fn get_id(&self, key: &str) -> Result<Uuid, AppError> {
        self.key_to_id
            .get(key)
            .copied()
            .ok_or_else(|| AppError::GraphMapper(format!("Key '{key}' not found in map.")))
    }
}
