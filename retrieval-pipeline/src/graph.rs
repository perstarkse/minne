use std::collections::{HashMap, HashSet};

use surrealdb::{sql::Thing, Error};

use common::storage::{
    db::SurrealDbClient,
    types::{
        knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
        StoredObject,
    },
};

/// Find entities related to the given entity via graph relationships.
///
/// Queries the `relates_to` edge table for all relationships involving the entity,
/// then fetches and returns the neighboring entities.
///
/// # Arguments
/// * `db` - Database client
/// * `entity_id` - ID of the entity to find neighbors for
/// * `user_id` - User ID for access control
/// * `limit` - Maximum number of neighbors to return
pub async fn find_entities_by_relationship_by_id(
    db: &SurrealDbClient,
    entity_id: &str,
    user_id: &str,
    limit: usize,
) -> Result<Vec<KnowledgeEntity>, Error> {
    let mut relationships_response = db
        .query(
            "
            SELECT * FROM relates_to
            WHERE metadata.user_id = $user_id
              AND (in = type::thing('knowledge_entity', $entity_id)
                   OR out = type::thing('knowledge_entity', $entity_id))
            ",
        )
        .bind(("entity_id", entity_id.to_owned()))
        .bind(("user_id", user_id.to_owned()))
        .await?;

    let relationships: Vec<KnowledgeRelationship> = relationships_response.take(0)?;
    if relationships.is_empty() {
        return Ok(Vec::new());
    }

    let mut neighbor_ids: Vec<String> = Vec::with_capacity(relationships.len());
    let mut seen: HashSet<String> = HashSet::with_capacity(relationships.len());
    for rel in relationships {
        if rel.in_ == entity_id {
            if seen.insert(rel.out.clone()) {
                neighbor_ids.push(rel.out);
            }
        } else if rel.out == entity_id {
            if seen.insert(rel.in_.clone()) {
                neighbor_ids.push(rel.in_);
            }
        } else {
            if seen.insert(rel.in_.clone()) {
                neighbor_ids.push(rel.in_.clone());
            }
            if seen.insert(rel.out.clone()) {
                neighbor_ids.push(rel.out);
            }
        }
    }

    neighbor_ids.retain(|id| id != entity_id);

    if neighbor_ids.is_empty() {
        return Ok(Vec::new());
    }

    if limit > 0 && neighbor_ids.len() > limit {
        neighbor_ids.truncate(limit);
    }

    let thing_ids: Vec<Thing> = neighbor_ids
        .iter()
        .map(|id| Thing::from((KnowledgeEntity::table_name(), id.as_str())))
        .collect();

    let mut neighbors_response = db
        .query("SELECT * FROM type::table($table) WHERE id IN $things AND user_id = $user_id")
        .bind(("table", KnowledgeEntity::table_name().to_owned()))
        .bind(("things", thing_ids))
        .bind(("user_id", user_id.to_owned()))
        .await?;

    let neighbors: Vec<KnowledgeEntity> = neighbors_response.take(0)?;
    if neighbors.is_empty() {
        return Ok(Vec::new());
    }

    let mut neighbor_map: HashMap<String, KnowledgeEntity> = neighbors
        .into_iter()
        .map(|entity| (entity.id.clone(), entity))
        .collect();

    let mut ordered = Vec::with_capacity(neighbor_ids.len());
    for id in neighbor_ids {
        if let Some(entity) = neighbor_map.remove(&id) {
            ordered.push(entity);
        }
        if limit > 0 && ordered.len() >= limit {
            break;
        }
    }

    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use anyhow::{self, Context};
    use super::*;
    use common::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};
    use common::storage::types::knowledge_relationship::KnowledgeRelationship;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_find_entities_by_relationship_by_id() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;

        let entity_type = KnowledgeEntityType::Document;
        let user_id = "user123".to_string();

        let central_entity = KnowledgeEntity::new(
            "central_source".to_string(),
            "Central Entity".to_string(),
            "Central Description".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        let related_entity1 = KnowledgeEntity::new(
            "related_source1".to_string(),
            "Related Entity 1".to_string(),
            "Related Description 1".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        let related_entity2 = KnowledgeEntity::new(
            "related_source2".to_string(),
            "Related Entity 2".to_string(),
            "Related Description 2".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        let unrelated_entity = KnowledgeEntity::new(
            "unrelated_source".to_string(),
            "Unrelated Entity".to_string(),
            "Unrelated Description".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        let central_entity = db
            .store_item(central_entity.clone())
            .await
            .with_context(|| "Failed to store central entity".to_string())?
            .ok_or_else(|| anyhow::anyhow!("Central entity not returned after store"))?;
        let related_entity1 = db
            .store_item(related_entity1.clone())
            .await
            .with_context(|| "Failed to store related entity 1".to_string())?
            .ok_or_else(|| anyhow::anyhow!("Related entity 1 not returned after store"))?;
        let related_entity2 = db
            .store_item(related_entity2.clone())
            .await
            .with_context(|| "Failed to store related entity 2".to_string())?
            .ok_or_else(|| anyhow::anyhow!("Related entity 2 not returned after store"))?;
        let _unrelated_entity = db
            .store_item(unrelated_entity.clone())
            .await
            .with_context(|| "Failed to store unrelated entity".to_string())?
            .ok_or_else(|| anyhow::anyhow!("Unrelated entity not returned after store"))?;

        let source_id = "relationship_source".to_string();

        let relationship1 = KnowledgeRelationship::new(
            central_entity.id.clone(),
            related_entity1.id.clone(),
            user_id.clone(),
            source_id.clone(),
            "references".to_string(),
        );

        let relationship2 = KnowledgeRelationship::new(
            central_entity.id.clone(),
            related_entity2.id.clone(),
            user_id.clone(),
            source_id.clone(),
            "contains".to_string(),
        );

        relationship1
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship 1".to_string())?;
        relationship2
            .store_relationship(&db)
            .await
            .with_context(|| "Failed to store relationship 2".to_string())?;

        let related_entities =
            find_entities_by_relationship_by_id(&db, &central_entity.id, &user_id, usize::MAX)
                .await
                .with_context(|| "Failed to find entities by relationship".to_string())?;

        assert!(
            related_entities.len() >= 2,
            "Should find related entities in both directions"
        );

        Ok(())
    }
}
