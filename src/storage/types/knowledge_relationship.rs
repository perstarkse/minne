use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};
use surrealdb::{engine::any::Any, sql::Subquery, Surreal};
use tracing::debug;
use uuid::Uuid;

stored_object!(KnowledgeRelationship, "relates_to", {
    #[serde(rename = "in")]
    in_: String,
    out: String,
    relationship_type: String,
    source_id: String,
    metadata: Option<serde_json::Value>
});

impl KnowledgeRelationship {
    pub fn new(
        in_: String,
        out: String,
        source_id: String,
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
            source_id,
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

    pub async fn delete_relationships_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE knowledge_entity -> relates_to WHERE source_id = '{}'",
            source_id
        );

        db_client.query(query).await?;

        Ok(())
    }
}
