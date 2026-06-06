use crate::storage::types::user::User;
use crate::utils::serde_helpers::deserialize_flexible_id;
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
    #[must_use]
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

    pub async fn store_relationship(self, db_client: &SurrealDbClient) -> Result<(), AppError> {
        User::get_and_validate_knowledge_entity(&self.in_, &self.metadata.user_id, db_client)
            .await?;
        User::get_and_validate_knowledge_entity(&self.out, &self.metadata.user_id, db_client)
            .await?;

        let Self {
            id,
            in_,
            out,
            metadata:
                RelationshipMetadata {
                    user_id,
                    source_id,
                    relationship_type,
                },
        } = self;

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
            .bind(("rel_id", id))
            .bind(("in_id", in_))
            .bind(("out_id", out))
            .bind(("user_id", user_id))
            .bind(("source_id", source_id))
            .bind(("relationship_type", relationship_type))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        Ok(())
    }

    pub async fn delete_relationships_by_source_id(
        source_id: &str,
        user_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        db_client
            .client
            .query(
                "DELETE FROM relates_to WHERE metadata.source_id = $source_id AND metadata.user_id = $user_id",
            )
            .bind(("source_id", source_id.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        Ok(())
    }

    pub async fn delete_relationship_by_id(
        id: &str,
        user_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let mut delete_result = db_client
            .client
            .query(
                "DELETE type::thing('relates_to', $id) WHERE metadata.user_id = $user_id RETURN BEFORE;",
            )
            .bind(("id", id.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .await
            .map_err(AppError::from)?;
        let deleted: Vec<KnowledgeRelationship> =
            delete_result.take(0).map_err(AppError::from)?;

        if !deleted.is_empty() {
            return Ok(());
        }

        let mut exists_result = db_client
            .client
            .query("SELECT * FROM type::thing('relates_to', $id)")
            .bind(("id", id.to_owned()))
            .await
            .map_err(AppError::from)?;
        let existing: Option<KnowledgeRelationship> =
            exists_result.take(0).map_err(AppError::from)?;

        if existing.is_some() {
            Err(AppError::Auth(
                "Not authorized to delete relationship".into(),
            ))
        } else {
            Err(AppError::NotFound(format!("Relationship {id} not found")))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use super::*;
    use crate::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};
    use anyhow::{self, Context};

    use crate::test_utils::setup_test_db;

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

    async fn create_test_entity(
        name: &str,
        user_id: &str,
        db_client: &SurrealDbClient,
    ) -> anyhow::Result<String> {
        let source_id = "source123".to_string();
        let description = format!("Description for {name}");
        let entity_type = KnowledgeEntityType::Document;

        let entity = KnowledgeEntity::new(
            source_id,
            name.to_string(),
            description,
            entity_type,
            None,
            user_id.to_string(),
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
        let db = setup_test_db().await?;
        let user_id = "user123";

        let entity1_id = create_test_entity("Entity 1", user_id, &db).await?;
        let entity2_id = create_test_entity("Entity 2", user_id, &db).await?;

        let source_id = "source123".to_string();
        let relationship_type = "references".to_string();

        let relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.to_string(),
            source_id.clone(),
            relationship_type,
        );
        let relationship_id = relationship.id.clone();

        relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship".to_string())?;

        let persisted = get_relationship_by_id(&relationship_id, &db)
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
    async fn test_store_relationship_rejects_foreign_entity() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let owner_entity = create_test_entity("Owner entity", "owner-user", &db).await?;
        let other_entity = create_test_entity("Other entity", "other-user", &db).await?;

        let relationship = KnowledgeRelationship::new(
            owner_entity,
            other_entity,
            "owner-user".to_string(),
            "source123".to_string(),
            "references".to_string(),
        );

        let result = relationship.store_relationship(&db).await;
        assert!(matches!(result, Err(AppError::Auth(_))));

        Ok(())
    }

    #[tokio::test]
    async fn test_store_relationship_resists_query_injection() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "user123";

        let entity1_id = create_test_entity("Entity 1", user_id, &db).await?;
        let entity2_id = create_test_entity("Entity 2", user_id, &db).await?;

        let relationship = KnowledgeRelationship::new(
            entity1_id,
            entity2_id,
            user_id.to_string(),
            "source123'; DELETE FROM relates_to; --".to_string(),
            "references'; UPDATE user SET admin = true; --".to_string(),
        );
        let relationship_id = relationship.id.clone();

        relationship
            .store_relationship(&db)
            .await
            .expect("store relationship should safely handle quote-containing values");

        let mut res = db
            .client
            .query("SELECT * FROM relates_to WHERE id = type::thing('relates_to', $id)")
            .bind(("id", relationship_id))
            .await
            .expect("query relationship by id failed");
        let rows: Vec<KnowledgeRelationship> = res.take(0).expect("take rows");

        assert_eq!(rows.len(), 1);
        let row = rows.first().expect("expected 1 row");
        assert_eq!(
            row.metadata.source_id,
            "source123'; DELETE FROM relates_to; --"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_store_and_delete_relationship() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "user123";

        let entity1_id = create_test_entity("Entity 1", user_id, &db).await?;
        let entity2_id = create_test_entity("Entity 2", user_id, &db).await?;

        let source_id = "source123".to_string();
        let relationship_type = "references".to_string();

        let relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.to_string(),
            source_id.clone(),
            relationship_type,
        );
        let relationship_id = relationship.id.clone();

        relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship".to_string())?;

        let mut existing_before_delete = db
            .query("SELECT * FROM relates_to WHERE metadata.user_id = $user_id AND metadata.source_id = $source_id")
            .bind(("user_id", user_id.to_string()))
            .bind(("source_id", source_id.clone()))
            .await
            .with_context(|| "Query failed".to_string())?;
        let before_results: Vec<KnowledgeRelationship> =
            existing_before_delete.take(0).unwrap_or_default();
        assert!(
            !before_results.is_empty(),
            "Relationship should exist before deletion"
        );

        KnowledgeRelationship::delete_relationship_by_id(&relationship_id, user_id, &db)
            .await
            .with_context(|| "Failed to delete relationship by ID".to_string())?;

        let mut result = db
            .query("SELECT * FROM relates_to WHERE metadata.user_id = $user_id AND metadata.source_id = $source_id")
            .bind(("user_id", user_id.to_string()))
            .bind(("source_id", source_id))
            .await
            .with_context(|| "Query failed".to_string())?;
        let results: Vec<KnowledgeRelationship> = result.take(0).unwrap_or_default();

        assert!(results.is_empty(), "Relationship should be deleted");

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_relationship_by_id_unauthorized() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let owner_user_id = "owner-user";

        let entity1_id = create_test_entity("Entity 1", owner_user_id, &db).await?;
        let entity2_id = create_test_entity("Entity 2", owner_user_id, &db).await?;

        let source_id = "source123".to_string();

        let relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            owner_user_id.to_string(),
            source_id,
            "references".to_string(),
        );
        let relationship_id = relationship.id.clone();

        relationship
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship".to_string())?;

        let mut before_attempt = db
            .query("SELECT * FROM relates_to WHERE metadata.user_id = $user_id")
            .bind(("user_id", owner_user_id.to_string()))
            .await
            .with_context(|| "Query failed".to_string())?;
        let before_results: Vec<KnowledgeRelationship> = before_attempt.take(0).unwrap_or_default();
        assert!(
            !before_results.is_empty(),
            "Relationship should exist before unauthorized delete attempt"
        );

        let result = KnowledgeRelationship::delete_relationship_by_id(
            &relationship_id,
            "different-user",
            &db,
        )
        .await;

        match result {
            Err(AppError::Auth(_)) => {}
            _ => anyhow::bail!(
                "Expected authorization error when deleting someone else's relationship"
            ),
        }

        let mut after_attempt = db
            .query("SELECT * FROM relates_to WHERE metadata.user_id = $user_id")
            .bind(("user_id", owner_user_id.to_string()))
            .await
            .with_context(|| "Query failed".to_string())?;
        let results: Vec<KnowledgeRelationship> = after_attempt.take(0).unwrap_or_default();

        assert!(
            !results.is_empty(),
            "Relationship should still exist after unauthorized delete attempt"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_store_relationship_exists() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "user123";

        let entity1_id = create_test_entity("Entity 1", user_id, &db).await?;
        let entity2_id = create_test_entity("Entity 2", user_id, &db).await?;
        let entity3_id = create_test_entity("Entity 3", user_id, &db).await?;

        let source_id = "source123".to_string();
        let different_source_id = "different_source".to_string();

        let relationship1 = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.to_string(),
            source_id.clone(),
            "references".to_string(),
        );

        let relationship2 = KnowledgeRelationship::new(
            entity2_id.clone(),
            entity3_id.clone(),
            user_id.to_string(),
            source_id.clone(),
            "contains".to_string(),
        );

        let different_relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity3_id.clone(),
            user_id.to_string(),
            different_source_id.clone(),
            "mentions".to_string(),
        );
        let relationship1_id = relationship1.id.clone();
        let relationship2_id = relationship2.id.clone();
        let different_relationship_id = different_relationship.id.clone();

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

        KnowledgeRelationship::delete_relationships_by_source_id(&source_id, user_id, &db)
            .await
            .with_context(|| "Failed to delete relationships by source_id".to_string())?;

        let result1 = get_relationship_by_id(&relationship1_id, &db).await;
        let result2 = get_relationship_by_id(&relationship2_id, &db).await;
        let different_result = get_relationship_by_id(&different_relationship_id, &db).await;

        assert!(result1.is_none(), "Relationship 1 should be deleted");
        assert!(result2.is_none(), "Relationship 2 should be deleted");
        let remaining =
            different_result.expect("Relationship with different source_id should remain");
        assert_eq!(remaining.metadata.source_id, different_source_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_relationships_by_source_id_scoped_to_user() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        let user_a = "user-a";
        let user_b = "user-b";
        let shared_source = "shared-source";

        let a1 = create_test_entity("A1", user_a, &db).await?;
        let a2 = create_test_entity("A2", user_a, &db).await?;
        let b1 = create_test_entity("B1", user_b, &db).await?;
        let b2 = create_test_entity("B2", user_b, &db).await?;

        let rel_a = KnowledgeRelationship::new(
            a1,
            a2,
            user_a.to_string(),
            shared_source.to_string(),
            "references".to_string(),
        );
        let rel_b = KnowledgeRelationship::new(
            b1,
            b2,
            user_b.to_string(),
            shared_source.to_string(),
            "references".to_string(),
        );
        let rel_a_id = rel_a.id.clone();
        let rel_b_id = rel_b.id.clone();

        rel_a.store_relationship(&db).await?;
        rel_b.store_relationship(&db).await?;

        KnowledgeRelationship::delete_relationships_by_source_id(shared_source, user_a, &db)
            .await?;

        assert!(get_relationship_by_id(&rel_a_id, &db).await.is_none());
        assert!(get_relationship_by_id(&rel_b_id, &db).await.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_relationships_by_source_id_resists_query_injection() -> anyhow::Result<()>
    {
        let db = setup_test_db().await?;
        let user_id = "user123";

        let entity1_id = create_test_entity("Entity 1", user_id, &db).await?;
        let entity2_id = create_test_entity("Entity 2", user_id, &db).await?;
        let entity3_id = create_test_entity("Entity 3", user_id, &db).await?;

        let safe_relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity2_id.clone(),
            user_id.to_string(),
            "safe_source".to_string(),
            "references".to_string(),
        );

        let other_relationship = KnowledgeRelationship::new(
            entity2_id,
            entity3_id,
            user_id.to_string(),
            "other_source".to_string(),
            "contains".to_string(),
        );
        let safe_relationship_id = safe_relationship.id.clone();
        let other_relationship_id = other_relationship.id.clone();

        safe_relationship
            .store_relationship(&db)
            .await
            .expect("store safe relationship");
        other_relationship
            .store_relationship(&db)
            .await
            .expect("store other relationship");

        KnowledgeRelationship::delete_relationships_by_source_id(
            "safe_source' OR 1=1 --",
            user_id,
            &db,
        )
        .await
        .expect("delete call should succeed");

        let remaining_safe = get_relationship_by_id(&safe_relationship_id, &db).await;
        let remaining_other = get_relationship_by_id(&other_relationship_id, &db).await;

        assert!(remaining_safe.is_some(), "Safe relationship should remain");
        assert!(
            remaining_other.is_some(),
            "Other relationship should remain"
        );

        Ok(())
    }
}
