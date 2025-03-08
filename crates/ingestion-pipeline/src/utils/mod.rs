pub mod llm_instructions;

use std::collections::HashMap;
use uuid::Uuid;

/// Intermediate struct to hold mapping between LLM keys and generated IDs.
#[derive(Clone)]
pub struct GraphMapper {
    pub key_to_id: HashMap<String, Uuid>,
}

impl Default for GraphMapper {
    fn default() -> Self {
        GraphMapper::new()
    }
}

impl GraphMapper {
    pub fn new() -> Self {
        GraphMapper {
            key_to_id: HashMap::new(),
        }
    }
    /// Get ID, tries to parse UUID
    pub fn get_or_parse_id(&mut self, key: &str) -> Uuid {
        if let Ok(parsed_uuid) = Uuid::parse_str(key) {
            parsed_uuid
        } else {
            *self.key_to_id.get(key).unwrap()
        }
    }

    /// Assigns a new UUID for a given key.
    pub fn assign_id(&mut self, key: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.key_to_id.insert(key.to_string(), id);
        id
    }

    /// Retrieves the UUID for a given key.
    pub fn get_id(&self, key: &str) -> Option<&Uuid> {
        self.key_to_id.get(key)
    }
}
