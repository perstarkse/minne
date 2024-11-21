use crate::stored_object;
use uuid::Uuid;

stored_object!(KnowledgeRelationship, "knowledge_relationship", {
    in_: String,
    out: String,
    relationship_type: String,
    metadata: Option<serde_json::Value>
});

impl KnowledgeRelationship {
    pub fn new(
        in_: String,
        out: String,
        relationship_type: String,
        metadata: Option<serde_json::Value>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            in_,
            out,
            relationship_type,
            metadata,
        }
    }
}
