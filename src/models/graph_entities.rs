use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

/// Represents a generic knowledge entity in the graph.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnowledgeEntity {
    pub id: Uuid, // Generated in Rust
    pub name: String,
    pub description: String,
    pub entity_type: KnowledgeEntityType,
    pub source_id: Option<Uuid>, // Links to FileInfo or TextContent
    pub metadata: Option<serde_json::Value>, // Additional metadata
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
    pub id: Uuid, // Generated in Rust
    #[serde(rename = "in")]
    pub in_: Uuid, // Target KnowledgeEntity ID
    pub out: Uuid, // Source KnowledgeEntity ID
    pub relationship_type: String, // e.g., RelatedTo, RelevantTo
    pub metadata: Option<serde_json::Value>, // Additional metadata
}

use std::collections::HashMap;

use crate::utils::llm::LLMGraphAnalysisResult;
use crate::utils::llm::LLMKnowledgeEntity;
use crate::utils::llm::LLMRelationship;

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

impl From<&LLMKnowledgeEntity> for KnowledgeEntity {
    fn from(llm_entity: &LLMKnowledgeEntity) -> Self {
        KnowledgeEntity {
            id: Uuid::new_v4(),
            name: llm_entity.name.clone(),
            description: llm_entity.description.clone(),
            entity_type: KnowledgeEntityType::from(llm_entity.entity_type.clone()),
            source_id: None, // To be linked externally if needed
            metadata: None,  // Populate if metadata is provided
        }
    }
}

impl From<&LLMRelationship> for KnowledgeRelationship {
    fn from(llm_rel: &LLMRelationship) -> Self {
        KnowledgeRelationship {
            id: Uuid::new_v4(),
            in_: Uuid::nil(), // Placeholder; to be set after mapping
            out: Uuid::nil(), // Placeholder; to be set after mapping
            relationship_type: llm_rel.type_.clone(),
            metadata: None, // Populate if metadata is provided
        }
    }
}
