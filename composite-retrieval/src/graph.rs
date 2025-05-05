use surrealdb::Error;
use tracing::debug;

use common::storage::{db::SurrealDbClient, types::knowledge_entity::KnowledgeEntity};

/// Retrieves database entries that match a specific source identifier.
///
/// This function queries the database for all records in a specified table that have
/// a matching `source_id` field. It's commonly used to find related entities or
/// track the origin of database entries.
///
/// # Arguments
///
/// * `source_id` - The identifier to search for in the database
/// * `table_name` - The name of the table to search in
/// * `db_client` - The SurrealDB client instance for database operations
///
/// # Type Parameters
///
/// * `T` - The type to deserialize the query results into. Must implement `serde::Deserialize`
///
/// # Returns
///
/// Returns a `Result` containing either:
/// * `Ok(Vec<T>)` - A vector of matching records deserialized into type `T`
/// * `Err(Error)` - An error if the database query fails
///
/// # Errors
///
/// This function will return a `Error` if:
/// * The database query fails to execute
/// * The results cannot be deserialized into type `T`
pub async fn find_entities_by_source_ids<T>(
    source_id: Vec<String>,
    table_name: String,
    db: &SurrealDbClient,
) -> Result<Vec<T>, Error>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let query = "SELECT * FROM type::table($table) WHERE source_id IN $source_ids";

    db.query(query)
        .bind(("table", table_name))
        .bind(("source_ids", source_id))
        .await?
        .take(0)
}

/// Find entities by their relationship to the id
pub async fn find_entities_by_relationship_by_id(
    db: &SurrealDbClient,
    entity_id: String,
) -> Result<Vec<KnowledgeEntity>, Error> {
    let query = format!(
        "SELECT *, <-> relates_to <-> knowledge_entity AS related FROM knowledge_entity:`{}`",
        entity_id
    );

    debug!("{}", query);

    db.query(query).await?.take(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};
    use common::storage::types::knowledge_relationship::KnowledgeRelationship;
    use common::storage::types::StoredObject;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_find_entities_by_source_ids() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create some test entities with different source_ids
        let source_id1 = "source123".to_string();
        let source_id2 = "source456".to_string();
        let source_id3 = "source789".to_string();

        let entity_type = KnowledgeEntityType::Document;
        let embedding = vec![0.1, 0.2, 0.3];
        let user_id = "user123".to_string();

        // Entity with source_id1
        let entity1 = KnowledgeEntity::new(
            source_id1.clone(),
            "Entity 1".to_string(),
            "Description 1".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Entity with source_id2
        let entity2 = KnowledgeEntity::new(
            source_id2.clone(),
            "Entity 2".to_string(),
            "Description 2".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Another entity with source_id1
        let entity3 = KnowledgeEntity::new(
            source_id1.clone(),
            "Entity 3".to_string(),
            "Description 3".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Entity with source_id3
        let entity4 = KnowledgeEntity::new(
            source_id3.clone(),
            "Entity 4".to_string(),
            "Description 4".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Store all entities
        db.store_item(entity1.clone())
            .await
            .expect("Failed to store entity 1");
        db.store_item(entity2.clone())
            .await
            .expect("Failed to store entity 2");
        db.store_item(entity3.clone())
            .await
            .expect("Failed to store entity 3");
        db.store_item(entity4.clone())
            .await
            .expect("Failed to store entity 4");

        // Test finding entities by multiple source_ids
        let source_ids = vec![source_id1.clone(), source_id2.clone()];
        let found_entities: Vec<KnowledgeEntity> =
            find_entities_by_source_ids(source_ids, KnowledgeEntity::table_name().to_string(), &db)
                .await
                .expect("Failed to find entities by source_ids");

        // Should find 3 entities (2 with source_id1, 1 with source_id2)
        assert_eq!(
            found_entities.len(),
            3,
            "Should find 3 entities with the specified source_ids"
        );

        // Check that entities with source_id1 and source_id2 are found
        let found_source_ids: Vec<String> =
            found_entities.iter().map(|e| e.source_id.clone()).collect();
        assert!(
            found_source_ids.contains(&source_id1),
            "Should find entities with source_id1"
        );
        assert!(
            found_source_ids.contains(&source_id2),
            "Should find entities with source_id2"
        );
        assert!(
            !found_source_ids.contains(&source_id3),
            "Should not find entities with source_id3"
        );

        // Test finding entities by a single source_id
        let single_source_id = vec![source_id1.clone()];
        let found_entities: Vec<KnowledgeEntity> = find_entities_by_source_ids(
            single_source_id,
            KnowledgeEntity::table_name().to_string(),
            &db,
        )
        .await
        .expect("Failed to find entities by single source_id");

        // Should find 2 entities with source_id1
        assert_eq!(
            found_entities.len(),
            2,
            "Should find 2 entities with source_id1"
        );

        // Check that all found entities have source_id1
        for entity in found_entities {
            assert_eq!(
                entity.source_id, source_id1,
                "All found entities should have source_id1"
            );
        }

        // Test finding entities with non-existent source_id
        let non_existent_source_id = vec!["non_existent_source".to_string()];
        let found_entities: Vec<KnowledgeEntity> = find_entities_by_source_ids(
            non_existent_source_id,
            KnowledgeEntity::table_name().to_string(),
            &db,
        )
        .await
        .expect("Failed to find entities by non-existent source_id");

        // Should find 0 entities
        assert_eq!(
            found_entities.len(),
            0,
            "Should find 0 entities with non-existent source_id"
        );
    }

    #[tokio::test]
    async fn test_find_entities_by_relationship_by_id() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create some test entities
        let entity_type = KnowledgeEntityType::Document;
        let embedding = vec![0.1, 0.2, 0.3];
        let user_id = "user123".to_string();

        // Create the central entity we'll query relationships for
        let central_entity = KnowledgeEntity::new(
            "central_source".to_string(),
            "Central Entity".to_string(),
            "Central Description".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Create related entities
        let related_entity1 = KnowledgeEntity::new(
            "related_source1".to_string(),
            "Related Entity 1".to_string(),
            "Related Description 1".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        let related_entity2 = KnowledgeEntity::new(
            "related_source2".to_string(),
            "Related Entity 2".to_string(),
            "Related Description 2".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Create an unrelated entity
        let unrelated_entity = KnowledgeEntity::new(
            "unrelated_source".to_string(),
            "Unrelated Entity".to_string(),
            "Unrelated Description".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        // Store all entities
        let central_entity = db
            .store_item(central_entity.clone())
            .await
            .expect("Failed to store central entity")
            .unwrap();
        let related_entity1 = db
            .store_item(related_entity1.clone())
            .await
            .expect("Failed to store related entity 1")
            .unwrap();
        let related_entity2 = db
            .store_item(related_entity2.clone())
            .await
            .expect("Failed to store related entity 2")
            .unwrap();
        let _unrelated_entity = db
            .store_item(unrelated_entity.clone())
            .await
            .expect("Failed to store unrelated entity")
            .unwrap();

        // Create relationships
        let source_id = "relationship_source".to_string();

        // Create relationship 1: central -> related1
        let relationship1 = KnowledgeRelationship::new(
            central_entity.id.clone(),
            related_entity1.id.clone(),
            user_id.clone(),
            source_id.clone(),
            "references".to_string(),
        );

        // Create relationship 2: central -> related2
        let relationship2 = KnowledgeRelationship::new(
            central_entity.id.clone(),
            related_entity2.id.clone(),
            user_id.clone(),
            source_id.clone(),
            "contains".to_string(),
        );

        // Store relationships
        relationship1
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship 1");
        relationship2
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship 2");

        // Test finding entities related to the central entity
        let related_entities = find_entities_by_relationship_by_id(&db, central_entity.id.clone())
            .await
            .expect("Failed to find entities by relationship");

        // Check that we found relationships
        assert!(related_entities.len() > 0, "Should find related entities");
    }
}
