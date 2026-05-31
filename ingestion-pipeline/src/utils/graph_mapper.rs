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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    #[test]
    fn assign_then_get_returns_same_id() {
        let mut mapper = GraphMapper::new();
        let assigned = mapper.assign_id("entity-key");
        assert_eq!(mapper.get_id("entity-key").expect("key present"), assigned);
    }

    #[test]
    fn get_id_for_unknown_key_errors() {
        let mapper = GraphMapper::new();
        assert!(matches!(
            mapper.get_id("missing"),
            Err(AppError::GraphMapper(_))
        ));
    }

    #[test]
    fn get_or_parse_id_parses_raw_uuid_without_lookup() {
        let mapper = GraphMapper::new();
        let raw = Uuid::new_v4();
        let resolved = mapper
            .get_or_parse_id(&raw.to_string())
            .expect("raw uuid parses");
        assert_eq!(resolved, raw);
    }

    #[test]
    fn get_or_parse_id_falls_back_to_map_for_keys() {
        let mut mapper = GraphMapper::new();
        let assigned = mapper.assign_id("alias");
        assert_eq!(
            mapper.get_or_parse_id("alias").expect("alias mapped"),
            assigned
        );
    }

    #[test]
    fn get_or_parse_id_errors_for_unknown_non_uuid_key() {
        let mapper = GraphMapper::new();
        assert!(matches!(
            mapper.get_or_parse_id("not-a-uuid-and-not-mapped"),
            Err(AppError::GraphMapper(_))
        ));
    }
}
