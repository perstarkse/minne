use crate::stored_object;
use uuid::Uuid;

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

stored_object!(KnowledgeEntity, "knowledge_entity", {
    source_id: String,
    name: String,
    description: String,
    entity_type: KnowledgeEntityType,
    metadata: Option<serde_json::Value>,
    embedding: Vec<f32>,
    user_id: String
});

impl KnowledgeEntity {
    pub fn new(
        source_id: String,
        name: String,
        description: String,
        entity_type: KnowledgeEntityType,
        metadata: Option<serde_json::Value>,
        embedding: Vec<f32>,
        user_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id,
            name,
            description,
            entity_type,
            metadata,
            embedding,
            user_id,
        }
    }
}
