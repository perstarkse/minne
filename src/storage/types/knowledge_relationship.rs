use crate::{error::AppError, stored_object};
use surrealdb::{engine::any::Any, Surreal};
use tracing::debug;
use uuid::Uuid;

stored_object!(KnowledgeRelationship, "knowledge_relationship", {
    #[serde(rename = "in")]
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
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            in_,
            out,
            relationship_type,
            metadata,
        }
    }
    pub async fn store_relationship(&self, db_client: &Surreal<Any>) -> Result<(), AppError> {
        let query = format!(
            "RELATE knowledge_entity:`{}` -> relates_to -> knowledge_entity:`{}`",
            self.in_, self.out
        );

        let result = db_client.query(query).await?;

        debug!("{:?}", result);

        Ok(())
    }
}
