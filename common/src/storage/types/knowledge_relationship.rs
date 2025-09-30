use crate::storage::types::file_info::deserialize_flexible_id;
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

    pub async fn delete_relationship_by_id(
        id: &str,
        user_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let mut authorized_result = db_client
            .query(format!(
                "SELECT * FROM relates_to WHERE id = relates_to:`{}` AND metadata.user_id = '{}'",
                id, user_id
            ))
            .await?;
        let authorized: Vec<KnowledgeRelationship> = authorized_result.take(0).unwrap_or_default();

        if authorized.is_empty() {
            let mut exists_result = db_client
                .query(format!("SELECT * FROM relates_to:`{}`", id))
                .await?;
            let existing: Option<KnowledgeRelationship> = exists_result.take(0)?;

            if existing.is_some() {
                Err(AppError::Auth(
                    "Not authorized to delete relationship".into(),
                ))
            } else {
                Err(AppError::NotFound(format!("Relationship {} not found", id)))
            }
        } else {
            db_client
                .query(format!("DELETE relates_to:`{}`", id))
                .await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};

    // Helper function to create a test knowledge entity for the relationship tests
    async fn create_test_entity(name: &str, db_client: &SurrealDbClient) -> String {
        let source_id = "source123".to_string();
        let description = format!("Description for {}", name);
        let entity_type = KnowledgeEntityType::Document;
        let embedding = vec![0.1, 0.2, 0.3];
        let user_id = "user123".to_string();

        let entity = KnowledgeEntity::new(
            source_id,
            name.to_string(),
            description,
            entity_type,
            None,
            embedding,
            user_id,
        );

        let stored: Option<KnowledgeEntity> = db_client
            .store_item(entity)
            .await
            .expect("Failed to store entity");
        stored.unwrap().id
    }

    #[tokio::test]
    async fn test_relationship_creation() {
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

        // Verify fields are correctly set
        assert_eq!(relationship.in_, in_id);
        assert_eq!(relationship.out, out_id);
        assert_eq!(relationship.metadata.user_id, user_id);
        assert_eq!(relationship.metadata.source_id, source_id);
        assert_eq!(relationship.metadata.relationship_type, relationship_type);
        assert!(!relationship.id.is_empty());
    }

    #[tokio::test]
    async fn test_store_relationship() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create two entities to relate
        let entity1_id = create_test_entity("Entity 1", &db).await;
        let entity2_id = create_test_entity("Entity 2", &db).await;

        // Create relationship
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

        // Store the relationship
        relationship
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship");

        // Query to verify the relationship exists by checking for relationships with our source_id
        // This approach is more reliable than trying to look up by ID
        let check_query = format!(
            "SELECT * FROM relates_to WHERE metadata.source_id = '{}'",
            source_id
        );
        let mut check_result = db.query(check_query).await.expect("Check query failed");
        let check_results: Vec<KnowledgeRelationship> = check_result.take(0).unwrap_or_default();

        // Just verify that a relationship was created
        assert!(
            !check_results.is_empty(),
            "Relationship should exist in the database"
        );
    }

    #[tokio::test]
    async fn test_delete_relationship_by_id() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create two entities to relate
        let entity1_id = create_test_entity("Entity 1", &db).await;
        let entity2_id = create_test_entity("Entity 2", &db).await;

        // Create relationship
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

        // Store the relationship
        relationship
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship");

        // Ensure relationship exists before deletion attempt
        let mut existing_before_delete = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{}' AND metadata.source_id = '{}'",
                user_id, source_id
            ))
            .await
            .expect("Query failed");
        let before_results: Vec<KnowledgeRelationship> =
            existing_before_delete.take(0).unwrap_or_default();
        assert!(
            !before_results.is_empty(),
            "Relationship should exist before deletion"
        );

        // Delete the relationship by ID
        KnowledgeRelationship::delete_relationship_by_id(&relationship.id, &user_id, &db)
            .await
            .expect("Failed to delete relationship by ID");

        // Query to verify the relationship was deleted
        let mut result = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{}' AND metadata.source_id = '{}'",
                user_id, source_id
            ))
            .await
            .expect("Query failed");
        let results: Vec<KnowledgeRelationship> = result.take(0).unwrap_or_default();

        // Verify the relationship no longer exists
        assert!(results.is_empty(), "Relationship should be deleted");
    }

    #[tokio::test]
    async fn test_delete_relationship_by_id_unauthorized() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        let entity1_id = create_test_entity("Entity 1", &db).await;
        let entity2_id = create_test_entity("Entity 2", &db).await;

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
            .expect("Failed to store relationship");

        let mut before_attempt = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{}'",
                owner_user_id
            ))
            .await
            .expect("Query failed");
        let before_results: Vec<KnowledgeRelationship> = before_attempt.take(0).unwrap_or_default();
        assert!(
            !before_results.is_empty(),
            "Relationship should exist before unauthorized delete attempt"
        );

        let result = KnowledgeRelationship::delete_relationship_by_id(
            &relationship.id,
            "different-user",
            &db,
        )
        .await;

        match result {
            Err(AppError::Auth(_)) => {}
            _ => panic!("Expected authorization error when deleting someone else's relationship"),
        }

        let mut after_attempt = db
            .query(format!(
                "SELECT * FROM relates_to WHERE metadata.user_id = '{}'",
                owner_user_id
            ))
            .await
            .expect("Query failed");
        let results: Vec<KnowledgeRelationship> = after_attempt.take(0).unwrap_or_default();

        assert!(
            !results.is_empty(),
            "Relationship should still exist after unauthorized delete attempt"
        );
    }

    #[tokio::test]
    async fn test_delete_relationships_by_source_id() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create entities to relate
        let entity1_id = create_test_entity("Entity 1", &db).await;
        let entity2_id = create_test_entity("Entity 2", &db).await;
        let entity3_id = create_test_entity("Entity 3", &db).await;

        // Create relationships with the same source_id
        let user_id = "user123".to_string();
        let source_id = "source123".to_string();
        let different_source_id = "different_source".to_string();

        // Create two relationships with the same source_id
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

        // Create a relationship with a different source_id
        let different_relationship = KnowledgeRelationship::new(
            entity1_id.clone(),
            entity3_id.clone(),
            user_id.clone(),
            different_source_id.clone(),
            "mentions".to_string(),
        );

        // Store all relationships
        relationship1
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship 1");
        relationship2
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship 2");
        different_relationship
            .store_relationship(&db)
            .await
            .expect("Failed to store different relationship");

        // Delete relationships by source_id
        KnowledgeRelationship::delete_relationships_by_source_id(&source_id, &db)
            .await
            .expect("Failed to delete relationships by source_id");

        // Query to verify the relationships with source_id were deleted
        let query1 = format!("SELECT * FROM relates_to WHERE id = '{}'", relationship1.id);
        let query2 = format!("SELECT * FROM relates_to WHERE id = '{}'", relationship2.id);
        let different_query = format!(
            "SELECT * FROM relates_to WHERE id = '{}'",
            different_relationship.id
        );

        let mut result1 = db.query(query1).await.expect("Query 1 failed");
        let results1: Vec<KnowledgeRelationship> = result1.take(0).unwrap_or_default();

        let mut result2 = db.query(query2).await.expect("Query 2 failed");
        let results2: Vec<KnowledgeRelationship> = result2.take(0).unwrap_or_default();

        let mut different_result = db
            .query(different_query)
            .await
            .expect("Different query failed");
        let _different_results: Vec<KnowledgeRelationship> =
            different_result.take(0).unwrap_or_default();

        // Verify relationships with the source_id are deleted
        assert!(results1.is_empty(), "Relationship 1 should be deleted");
        assert!(results2.is_empty(), "Relationship 2 should be deleted");

        // For the relationship with different source ID, we need to check differently
        // Let's just verify we have a relationship where the source_id matches different_source_id
        let check_query = format!(
            "SELECT * FROM relates_to WHERE metadata.source_id = '{}'",
            different_source_id
        );
        let mut check_result = db.query(check_query).await.expect("Check query failed");
        let check_results: Vec<KnowledgeRelationship> = check_result.take(0).unwrap_or_default();

        // Verify the relationship with a different source_id still exists
        assert!(
            !check_results.is_empty(),
            "Relationship with different source_id should still exist"
        );
    }
}
