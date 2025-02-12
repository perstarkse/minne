use crate::storage::types::file_info::deserialize_flexible_id;
use crate::{error::AppError, storage::db::SurrealDbClient};
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RelationshipMetadata {
    pub user_id: String,
    pub source_id: String,
    pub relationship_type: String,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KnowledgeRelationship {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    #[serde(rename = "in", deserialize_with = "deserialize_flexible_id")]
    pub in_: String,
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub out: String,
    pub metadata: RelationshipMetadata,
}

impl KnowledgeRelationship {
    pub fn new(
        in_: String,
        out: String,
        user_id: String,
        source_id: String,
        relationship_type: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            in_,
            out,
            metadata: RelationshipMetadata {
                user_id,
                source_id,
                relationship_type,
            },
        }
    }
    pub async fn store_relationship(&self, db_client: &Surreal<Any>) -> Result<(), AppError> {
        let query = format!(
            r#"RELATE knowledge_entity:`{}`->relates_to:`{}`->knowledge_entity:`{}`
            SET
                metadata.user_id = '{}',
                metadata.source_id = '{}',
                metadata.relationship_type = '{}'"#,
            self.in_,
            self.id,
            self.out,
            self.metadata.user_id,
            self.metadata.source_id,
            self.metadata.relationship_type
        );

        db_client.query(query).await?;

        Ok(())
    }

    pub async fn delete_relationships_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE knowledge_entity -> relates_to WHERE metadata.source_id = '{}'",
            source_id
        );

        db_client.query(query).await?;

        Ok(())
    }
}
