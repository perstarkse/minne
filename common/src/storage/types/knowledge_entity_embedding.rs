use std::collections::HashMap;

use surrealdb::RecordId;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

stored_object!(KnowledgeEntityEmbedding, "knowledge_entity_embedding", {
    entity_id: RecordId,
    embedding: Vec<f32>,
    /// Denormalized user id for query scoping
    user_id: String
});

impl KnowledgeEntityEmbedding {
    /// Recreate the HNSW index with a new embedding dimension.
    pub async fn redefine_hnsw_index(
        db: &SurrealDbClient,
        dimension: usize,
    ) -> Result<(), AppError> {
        let query = format!(
            "BEGIN TRANSACTION;
             REMOVE INDEX IF EXISTS idx_embedding_knowledge_entity_embedding ON TABLE {table};
             DEFINE INDEX idx_embedding_knowledge_entity_embedding ON TABLE {table} FIELDS embedding HNSW DIMENSION {dimension};
             COMMIT TRANSACTION;",
            table = Self::table_name(),
        );

        let res = db.client.query(query).await.map_err(AppError::Database)?;
        res.check().map_err(AppError::Database)?;

        Ok(())
    }

    /// Create a new knowledge entity embedding
    pub fn new(entity_id: &str, embedding: Vec<f32>, user_id: String) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            entity_id: RecordId::from_table_key("knowledge_entity", entity_id),
            embedding,
            user_id,
        }
    }

    /// Get embedding by entity ID
    pub async fn get_by_entity_id(
        entity_id: &RecordId,
        db: &SurrealDbClient,
    ) -> Result<Option<Self>, AppError> {
        let query = format!(
            "SELECT * FROM {} WHERE entity_id = $entity_id LIMIT 1",
            Self::table_name()
        );
        let mut result = db
            .client
            .query(query)
            .bind(("entity_id", entity_id.clone()))
            .await
            .map_err(AppError::Database)?;
        let embeddings: Vec<Self> = result.take(0).map_err(AppError::Database)?;
        Ok(embeddings.into_iter().next())
    }

    /// Get embeddings for multiple entities in batch
    pub async fn get_by_entity_ids(
        entity_ids: &[RecordId],
        db: &SurrealDbClient,
    ) -> Result<HashMap<String, Vec<f32>>, AppError> {
        if entity_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let ids_list: Vec<RecordId> = entity_ids.iter().cloned().collect();

        let query = format!(
            "SELECT * FROM {} WHERE entity_id INSIDE $entity_ids",
            Self::table_name()
        );
        let mut result = db
            .client
            .query(query)
            .bind(("entity_ids", ids_list))
            .await
            .map_err(AppError::Database)?;
        let embeddings: Vec<Self> = result.take(0).map_err(AppError::Database)?;

        Ok(embeddings
            .into_iter()
            .map(|e| (e.entity_id.key().to_string(), e.embedding))
            .collect())
    }

    /// Delete embedding by entity ID
    pub async fn delete_by_entity_id(
        entity_id: &RecordId,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE FROM {} WHERE entity_id = $entity_id",
            Self::table_name()
        );
        db.client
            .query(query)
            .bind(("entity_id", entity_id.clone()))
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    /// Delete embeddings by source_id (via joining to knowledge_entity table)
    pub async fn delete_by_source_id(
        source_id: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = "SELECT id FROM knowledge_entity WHERE source_id = $source_id";
        let mut res = db
            .client
            .query(query)
            .bind(("source_id", source_id.to_owned()))
            .await
            .map_err(AppError::Database)?;
        #[derive(Deserialize)]
        struct IdRow {
            id: RecordId,
        }
        let ids: Vec<IdRow> = res.take(0).map_err(AppError::Database)?;

        for row in ids {
            Self::delete_by_entity_id(&row.id, db).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::SurrealDbClient;
    use crate::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};
    use chrono::Utc;
    use surrealdb::Value as SurrealValue;
    use uuid::Uuid;

    async fn setup_test_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, &database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        db
    }

    fn build_knowledge_entity_with_id(
        key: &str,
        source_id: &str,
        user_id: &str,
    ) -> KnowledgeEntity {
        KnowledgeEntity {
            id: key.to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_id: source_id.to_owned(),
            name: "Test entity".to_owned(),
            description: "Desc".to_owned(),
            entity_type: KnowledgeEntityType::Document,
            metadata: None,
            user_id: user_id.to_owned(),
        }
    }

    #[tokio::test]
    async fn test_create_and_get_by_entity_id() {
        let db = setup_test_db().await;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("set test index dimension");
        let user_id = "user_ke";
        let entity_key = "entity-1";
        let source_id = "source-ke";

        let embedding_vec = vec![0.11_f32, 0.22, 0.33];
        let entity = build_knowledge_entity_with_id(entity_key, source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), embedding_vec.clone(), &db)
            .await
            .expect("Failed to store entity with embedding");

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);

        let fetched = KnowledgeEntityEmbedding::get_by_entity_id(&entity_rid, &db)
            .await
            .expect("Failed to get embedding by entity_id")
            .expect("Expected embedding to exist");

        assert_eq!(fetched.user_id, user_id);
        assert_eq!(fetched.entity_id, entity_rid);
        assert_eq!(fetched.embedding, embedding_vec);
    }

    #[tokio::test]
    async fn test_delete_by_entity_id() {
        let db = setup_test_db().await;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("set test index dimension");
        let user_id = "user_ke";
        let entity_key = "entity-delete";
        let source_id = "source-del";

        let entity = build_knowledge_entity_with_id(entity_key, source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), vec![0.5_f32, 0.6, 0.7], &db)
            .await
            .expect("Failed to store entity with embedding");

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);

        let existing = KnowledgeEntityEmbedding::get_by_entity_id(&entity_rid, &db)
            .await
            .expect("Failed to get embedding before delete");
        assert!(existing.is_some());

        KnowledgeEntityEmbedding::delete_by_entity_id(&entity_rid, &db)
            .await
            .expect("Failed to delete by entity_id");

        let after = KnowledgeEntityEmbedding::get_by_entity_id(&entity_rid, &db)
            .await
            .expect("Failed to get embedding after delete");
        assert!(after.is_none());
    }

    #[tokio::test]
    async fn test_store_with_embedding_creates_entity_and_embedding() {
        let db = setup_test_db().await;
        let user_id = "user_store";
        let source_id = "source_store";
        let embedding = vec![0.2_f32, 0.3, 0.4];

        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, embedding.len())
            .await
            .expect("set test index dimension");

        let entity = build_knowledge_entity_with_id("entity-store", source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), embedding.clone(), &db)
            .await
            .expect("Failed to store entity with embedding");

        let stored_entity: Option<KnowledgeEntity> = db.get_item(&entity.id).await.unwrap();
        assert!(stored_entity.is_some());

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);
        let stored_embedding = KnowledgeEntityEmbedding::get_by_entity_id(&entity_rid, &db)
            .await
            .expect("Failed to fetch embedding");
        assert!(stored_embedding.is_some());
        let stored_embedding = stored_embedding.unwrap();
        assert_eq!(stored_embedding.user_id, user_id);
        assert_eq!(stored_embedding.entity_id, entity_rid);
    }

    #[tokio::test]
    async fn test_delete_by_source_id() {
        let db = setup_test_db().await;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("set test index dimension");
        let user_id = "user_ke";
        let source_id = "shared-ke";
        let other_source = "other-ke";

        let entity1 = build_knowledge_entity_with_id("entity-s1", source_id, user_id);
        let entity2 = build_knowledge_entity_with_id("entity-s2", source_id, user_id);
        let entity_other = build_knowledge_entity_with_id("entity-other", other_source, user_id);

        KnowledgeEntity::store_with_embedding(entity1.clone(), vec![1.0_f32, 1.1, 1.2], &db)
            .await
            .expect("Failed to store entity with embedding");
        KnowledgeEntity::store_with_embedding(entity2.clone(), vec![2.0_f32, 2.1, 2.2], &db)
            .await
            .expect("Failed to store entity with embedding");
        KnowledgeEntity::store_with_embedding(entity_other.clone(), vec![3.0_f32, 3.1, 3.2], &db)
            .await
            .expect("Failed to store entity with embedding");

        let entity1_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity1.id);
        let entity2_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity2.id);
        let other_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity_other.id);

        KnowledgeEntityEmbedding::delete_by_source_id(source_id, &db)
            .await
            .expect("Failed to delete by source_id");

        assert!(
            KnowledgeEntityEmbedding::get_by_entity_id(&entity1_rid, &db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            KnowledgeEntityEmbedding::get_by_entity_id(&entity2_rid, &db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(KnowledgeEntityEmbedding::get_by_entity_id(&other_rid, &db)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_redefine_hnsw_index_updates_dimension() {
        let db = setup_test_db().await;

        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 16)
            .await
            .expect("failed to redefine index");

        let mut info_res = db
            .client
            .query("INFO FOR TABLE knowledge_entity_embedding;")
            .await
            .expect("info query failed");
        let info: SurrealValue = info_res.take(0).expect("failed to take info result");
        let info_json: serde_json::Value =
            serde_json::to_value(info).expect("failed to convert info to json");
        let idx_sql = info_json["Object"]["indexes"]["Object"]
            ["idx_embedding_knowledge_entity_embedding"]["Strand"]
            .as_str()
            .unwrap_or_default();

        assert!(
            idx_sql.contains("DIMENSION 16"),
            "expected index definition to contain new dimension, got: {idx_sql}"
        );
    }

    #[tokio::test]
    async fn test_fetch_entity_via_record_id() {
        let db = setup_test_db().await;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("set test index dimension");
        let user_id = "user_ke";
        let entity_key = "entity-fetch";
        let source_id = "source-fetch";

        let entity = build_knowledge_entity_with_id(entity_key, source_id, user_id);
        KnowledgeEntity::store_with_embedding(entity.clone(), vec![0.7_f32, 0.8, 0.9], &db)
            .await
            .expect("Failed to store entity with embedding");

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);

        #[derive(Deserialize)]
        struct Row {
            entity_id: KnowledgeEntity,
        }

        let mut res = db
            .client
            .query(
                "SELECT entity_id FROM knowledge_entity_embedding WHERE entity_id = $id FETCH entity_id;",
            )
            .bind(("id", entity_rid.clone()))
            .await
            .expect("failed to fetch embedding with FETCH");
        let rows: Vec<Row> = res.take(0).expect("failed to deserialize fetch rows");

        assert_eq!(rows.len(), 1);
        let fetched_entity = &rows[0].entity_id;
        assert_eq!(fetched_entity.id, entity_key);
        assert_eq!(fetched_entity.name, "Test entity");
        assert_eq!(fetched_entity.user_id, user_id);
    }
}
