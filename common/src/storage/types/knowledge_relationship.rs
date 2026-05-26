use crate::storage::types::serde_helpers::deserialize_flexible_id;
use crate::{error::AppError, storage::db::SurrealDbClient};
use serde::{Deserialize, Serialize};
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
    pub async fn store_relationship(&self, db_client: &SurrealDbClient) -> Result<(), AppError> {
        db_client
            .client
            .query(
                r#"BEGIN TRANSACTION;
                LET $in_entity = type::thing('knowledge_entity', $in_id);
                LET $out_entity = type::thing('knowledge_entity', $out_id);
                LET $relation = type::thing('relates_to', $rel_id);
                DELETE type::thing('relates_to', $rel_id);
                RELATE $in_entity->$relation->$out_entity SET
                    metadata.user_id = $user_id,
                    metadata.source_id = $source_id,
                    metadata.relationship_type = $relationship_type;
                COMMIT TRANSACTION;"#,
            )
            .bind(("rel_id", self.id.clone()))
            .bind(("in_id", self.in_.clone()))
            .bind(("out_id", self.out.clone()))
            .bind(("user_id", self.metadata.user_id.clone()))
            .bind(("source_id", self.metadata.source_id.clone()))
            .bind(("relationship_type", self.metadata.relationship_type.clone()))
            .await?
            .check()?;

        Ok(())
    }

    pub async fn delete_relationships_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        db_client
            .client
            .query("DELETE FROM relates_to WHERE metadata.source_id = $source_id")
            .bind(("source_id", source_id.to_owned()))
            .await?
            .check()?;

        Ok(())
    }

    pub async fn delete_relationship_by_id(
        id: &str,
        user_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let mut authorized_result = db_client
            .client
            .query(
                "SELECT * FROM relates_to WHERE id = type::thing('relates_to', $id) AND metadata.user_id = $user_id",
            )
            .bind(("id", id.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .await?;
        let authorized: Vec<KnowledgeRelationship> = authorized_result.take(0).unwrap_or_default();

        if authorized.is_empty() {
            let mut exists_result = db_client
                .client
                .query("SELECT * FROM type::thing('relates_to', $id)")
                .bind(("id", id.to_owned()))
                .await?;
            let existing: Option<KnowledgeRelationship> = exists_result.take(0)?;

            if existing.is_some() {
                Err(AppError::Auth(
                    "Not authorized to delete relationship".into(),
                ))
            } else {
                Err(AppError::NotFound(format!("Relationship {id} not found")))
            }
        } else {
            db_client
                .client
                .query("DELETE type::thing('relates_to', $id)")
                .bind(("id", id.to_owned()))
                .await?
                .check()?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{self, Context};
    use super::*;
    use crate::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};

    async fn setup_test_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        db
    }

    async fn get_relationship_by_id(
        relationship_id: &str,
        db_client: &SurrealDbClient,
    ) -> Option<KnowledgeRelationship> {
        let mut result = db_client
            .client
            .query("SELECT * FROM type::thing('relates_to', $id)")
            .bind(("id", relationship_id.to_owned()))
            .await
            .expect("relationship query by id failed");

        result.take(0).expect("failed to take relationship by id")
    }

    async fn create_test_entity(name: &str, db_client: &SurrealDbClient) -> anyhow::Result<String> {
        let source_id = "source123".to_string();
        let description = format!("Description for {name}");
        let entity_type = KnowledgeEntityType::Document;
        let user_id = "user123".to_string();

        let entity = KnowledgeEntity::new(
            source_id,
            name.to_string(),
            description,
            entity_type,
            None,
            user_id,
        );

        let stored: Option<KnowledgeEntity> = db_client
            .store_item(entity)
            .await
            .with_context(|| "Failed to store entity".to_string())?;
        stored
            .ok_or_else(|| anyhow::anyhow!("Expected stored entity to return Some"))
            .map(|e| e.id)
    }

    #[tokio::test]
    async fn test_relationship_creation() -> anyhow::Result<()> {
        let in_id = "entity1".to_string();
        let out_id = "entity2".to_string();
        let user_id = "user123".to_string();
        let source_id = "source123".to_string();
        let relationship_type = "references".to_string();

        let relationship = KnowledgeRelationship::new(
            in_id.clone(),
            out_id.clone(),
            user_id.clone(),
            source_id.clone(),
            relationship_type.clone(),
        );

        assert_eq!(relationship.in_, in_id);
        assert_eq!(relationship.out, out_id);
        assert_eq!(relationship.metadata.user_id, user_id);
        assert_eq!(relationship.metadata.source_id, source_id);
        assert_eq!(relationship.metadata.relationship_type, relationship_type);
        assert!(!relationship.id.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_store_and_verify_by_source_id() -> anyhow::Result<()> {
        let db = setup_test_db().await;

        let entity1_id = create_test_entity("Entity 1", &db).await?;
        let entity2_id = create_test_entity("Entity 2", &db).await?;

        let user_id = "user123".to_string();
        let source_id = "source123".to_string();
        let relationship_type = "references".to_string();

        let relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.clone(),
            source_id.clone(),
            relationship_type,
        );

        relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship".to_string())?;

        let persisted = get_relationship_by_id(&relationship.id, &db)
            .await
            .expect("Relationship should be retrievable by id");
        assert_eq!(persisted.in_, entity1_id);
        assert_eq!(persisted.out, entity2_id);
        assert_eq!(persisted.metadata.user_id, user_id);
        assert_eq!(persisted.metadata.source_id, source_id);

        let mut check_result = db
            .query("SELECT * FROM relates_to WHERE metadata.source_id = $source_id")
            .bind(("source_id", source_id.clone()))
            .await
            .expect("Check query failed");
        let check_results: Vec<KnowledgeRelationship> = check_result.take(0).unwrap_or_default();

        assert_eq!(
            check_results.len(),
            1,
            "Expected one relationship for source_id"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_store_relationship_resists_query_injection() -> anyhow::Result<()> {
        let db = setup_test_db().await;

        let entity1_id = create_test_entity("Entity 1", &db).await?;
        let entity2_id = create_test_entity("Entity 2", &db).await?;

        let relationship = KnowledgeRelationship::new(
            entity1_id,
            entity2_id,
            "user'123".to_string(),
            "source123'; DELETE FROM relates_to; --".to_string(),
            "references'; UPDATE user SET admin = true; --".to_string(),
        );

        relationship
            .store_relationship(&db)
            .await
            .expect("store relationship should safely handle quote-containing values");

        let mut res = db
            .client
            .query("SELECT * FROM relates_to WHERE id = type::thing('relates_to', $id)")
            .bind(("id", relationship.id.clone()))
            .await
            .expect("query relationship by id failed");
        let rows: Vec<KnowledgeRelationship> = res.take(0).expect("take rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].metadata.source_id,
            "source123'; DELETE FROM relates_to; --"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_store_and_delete_relationship() -> anyhow::Result<()> {
        let db = setup_test_db().await;

        let entity1_id = create_test_entity("Entity 1", &db).await?;
        let entity2_id = create_test_entity("Entity 2", &db).await?;

        let user_id = "user123".to_string();
        let source_id = "source123".to_string();
        let relationship_type = "references".to_string();

        let relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.clone(),
            source_id.clone(),
            relationship_type,
        );

        relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship".to_string())?;

        let mut existing_before_delete = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{user_id}' AND metadata.source_id = '{source_id}'"
            ))
            .await
            .with_context(|| "Query failed".to_string())?;
        let before_results: Vec<KnowledgeRelationship> =
            existing_before_delete.take(0).unwrap_or_default();
        assert!(!before_results.is_empty(), "Relationship should exist before deletion");

        KnowledgeRelationship::delete_relationship_by_id(&relationship.id, &user_id, &db)
            .await
            .with_context(|| "Failed to delete relationship by ID".to_string())?;

        let mut result = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{user_id}' AND metadata.source_id = '{source_id}'"
            ))
            .await
            .with_context(|| "Query failed".to_string())?;
        let results: Vec<KnowledgeRelationship> = result.take(0).unwrap_or_default();

        assert!(results.is_empty(), "Relationship should be deleted");

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_relationship_by_id_unauthorized() -> anyhow::Result<()> {
        let db = setup_test_db().await;

        let entity1_id = create_test_entity("Entity 1", &db).await?;
        let entity2_id = create_test_entity("Entity 2", &db).await?;

        let owner_user_id = "owner-user".to_string();
        let source_id = "source123".to_string();

        let relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            owner_user_id.clone(),
            source_id,
            "references".to_string(),
        );

        relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship".to_string())?;

        let mut before_attempt = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{owner_user_id}'"
            ))
            .await
            .with_context(|| "Query failed".to_string())?;
        let before_results: Vec<KnowledgeRelationship> = before_attempt.take(0).unwrap_or_default();
        assert!(!before_results.is_empty(), "Relationship should exist before unauthorized delete attempt");

        let result = KnowledgeRelationship::delete_relationship_by_id(
            &relationship.id,
            "different-user",
            &db,
        )
        .await;

        match result {
            Err(AppError::Auth(_)) => {}
            _ => anyhow::bail!("Expected authorization error when deleting someone else's relationship"),
        }

        let mut after_attempt = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{owner_user_id}'"
            ))
            .await
            .with_context(|| "Query failed".to_string())?;
        let results: Vec<KnowledgeRelationship> = after_attempt.take(0).unwrap_or_default();

        assert!(!results.is_empty(), "Relationship should still exist after unauthorized delete attempt");

        Ok(())
    }

    #[tokio::test]
    async fn test_store_relationship_exists() -> anyhow::Result<()> {
        let db = setup_test_db().await;

        let entity1_id = create_test_entity("Entity 1", &db).await?;
        let entity2_id = create_test_entity("Entity 2", &db).await?;
        let entity3_id = create_test_entity("Entity 3", &db).await?;

        let user_id = "user123".to_string();
        let source_id = "source123".to_string();
        let different_source_id = "different_source".to_string();

        let relationship1 = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.clone(),
            source_id.clone(),
            "references".to_string(),
        );

        let relationship2 = KnowledgeRelationship::new(
            entity2_id.clone(),
            entity3_id.clone(),
            user_id.clone(),
            source_id.clone(),
            "contains".to_string(),
        );

        let different_relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity3_id.clone(),
            user_id.clone(),
            different_source_id.clone(),
            "mentions".to_string(),
        );

        relationship1
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship 1".to_string())?;
        relationship2
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship 2".to_string())?;
        different_relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store different relationship".to_string())?;

        let mut before_delete = db
            .query("SELECT * FROM relates_to WHERE metadata.source_id = $source_id")
            .bind(("source_id", source_id.clone()))
            .await
            .expect("before delete query failed");
        let before_delete_rows: Vec<KnowledgeRelationship> =
            before_delete.take(0).unwrap_or_default();
        assert_eq!(before_delete_rows.len(), 2);

        let mut before_delete_different = db
            .query("SELECT * FROM relates_to WHERE metadata.source_id = $source_id")
            .bind(("source_id", different_source_id.clone()))
            .await
            .expect("before delete different query failed");
        let before_delete_different_rows: Vec<KnowledgeRelationship> =
            before_delete_different.take(0).unwrap_or_default();
        assert_eq!(before_delete_different_rows.len(), 1);

        KnowledgeRelationship::delete_relationships_by_source_id(&source_id, &db)
            .await
            .with_context(|| "Failed to delete relationships by source_id".to_string())?;

        let result1 = get_relationship_by_id(&relationship1.id, &db).await;
        let result2 = get_relationship_by_id(&relationship2.id, &db).await;
        let different_result = get_relationship_by_id(&different_relationship.id, &db).await;

        assert!(result1.is_none(), "Relationship 1 should be deleted");
        assert!(result2.is_none(), "Relationship 2 should be deleted");
        let remaining =
            different_result.expect("Relationship with different source_id should remain");
        assert_eq!(remaining.metadata.source_id, different_source_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_relationships_by_source_id_resists_query_injection() -> anyhow::Result<()> {
        let db = setup_test_db().await;

        let entity1_id = create_test_entity("Entity 1", &db).await?;
        let entity2_id = create_test_entity("Entity 2", &db).await?;
        let entity3_id = create_test_entity("Entity 3", &db).await?;

        let safe_relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            "user123".to_string(),
            "safe_source".to_string(),
            "references".to_string(),
        );

        let other_relationship = KnowledgeRelationship::new(
            entity2_id,
            entity3_id,
            "user123".to_string(),
            "other_source".to_string(),
            "contains".to_string(),
        );

        safe_relationship
            .store_relationship(&db)
            .await
            .expect("store safe relationship");
        other_relationship
            .store_relationship(&db)
            .await
            .expect("store other relationship");

        KnowledgeRelationship::delete_relationships_by_source_id("safe_source' OR 1=1 --", &db)
            .await
            .expect("delete call should succeed");

        let remaining_safe = get_relationship_by_id(&safe_relationship.id, &db).await;
        let remaining_other = get_relationship_by_id(&other_relationship.id, &db).await;

        assert!(remaining_safe.is_some(), "Safe relationship should remain");
        assert!(
            remaining_other.is_some(),
            "Other relationship should remain"
        );

        Ok(())
    }
}
