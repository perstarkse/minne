use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use std::collections::HashMap;
use surrealdb::sql::Thing;
use uuid::Uuid;

/// Represents a generic knowledge entity in the graph.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnowledgeEntity {
    #[serde(deserialize_with = "thing_to_string")]
    pub id: String,
    pub name: String,
    pub description: String,
    pub entity_type: KnowledgeEntityType,
    pub source_id: String,
    pub metadata: Option<serde_json::Value>,
}

fn thing_to_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let thing = Thing::deserialize(deserializer)?;
    Ok(thing.id.to_raw())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum KnowledgeEntityType {
    Idea,
    Project,
    Document,
    Page,
    TextSnippet,
    // Add more types as needed
}

impl From<String> for KnowledgeEntityType {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "idea" => KnowledgeEntityType::Idea,
            "project" => KnowledgeEntityType::Project,
            "document" => KnowledgeEntityType::Document,
            "page" => KnowledgeEntityType::Page,
            "textsnippet" => KnowledgeEntityType::TextSnippet,
            _ => KnowledgeEntityType::Document, // Default case
        }
    }
}

/// Represents a relationship between two knowledge entities.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnowledgeRelationship {
    #[serde(deserialize_with = "thing_to_string")]
    pub id: String,
    #[serde(rename = "in")]
    pub in_: String, // Target KnowledgeEntity ID
    pub out: String,                         // Source KnowledgeEntity ID
    pub relationship_type: String,           // e.g., RelatedTo, RelevantTo
    pub metadata: Option<serde_json::Value>, // Additional metadata
}

/// Intermediate struct to hold mapping between LLM keys and generated IDs.
pub struct GraphMapper {
    pub key_to_id: HashMap<String, Uuid>,
}

impl GraphMapper {
    pub fn new() -> Self {
        GraphMapper {
            key_to_id: HashMap::new(),
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
