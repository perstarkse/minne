use std::collections::HashMap;

use surrealdb::RecordId;

use crate::{
    error::AppError,
    storage::{db::SurrealDbClient, types::EmbeddingRecord},
    stored_object,
};

stored_object!(KnowledgeEntityEmbedding, "knowledge_entity_embedding", {
    entity_id: RecordId,
    embedding: Vec<f32>,
    /// Denormalized source id for bulk deletes
    source_id: String,
    /// Denormalized user id for query scoping
    user_id: String
});

impl EmbeddingRecord for KnowledgeEntityEmbedding {
    fn link_field() -> &'static str {
        "entity_id"
    }

    fn index_name() -> &'static str {
        "idx_embedding_knowledge_entity_embedding"
    }

    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn embedding(&self) -> &[f32] {
        &self.embedding
    }

    fn new(
        entity_id: &str,
        source_id: String,
        embedding: Vec<f32>,
        user_id: String,
        entity_table: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: entity_id.to_owned(),
            created_at: now,
            updated_at: now,
            entity_id: RecordId::from_table_key(entity_table, entity_id),
            embedding,
            source_id,
            user_id,
        }
    }
}

impl KnowledgeEntityEmbedding {
    /// Get embeddings for multiple entities in batch
    pub async fn get_by_entity_ids(
        entity_ids: &[RecordId],
        db: &SurrealDbClient,
    ) -> Result<HashMap<String, Vec<f32>>, AppError> {
        if entity_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = format!(
            "SELECT * FROM {} WHERE entity_id INSIDE $entity_ids",
            Self::table_name()
        );
        let mut result = db
            .client
            .query(query)
            .bind(("entity_ids", entity_ids.to_vec()))
            .await
            .map_err(AppError::from)?;
        let embeddings: Vec<Self> = result.take(0).map_err(AppError::from)?;

        Ok(embeddings
            .into_iter()
            .map(|e| (e.entity_id.key().to_string(), e.embedding))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use super::*;
    use crate::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};
    use crate::test_utils::{prepare_knowledge_entity_test_db, setup_test_db};
    use anyhow::{self, Context};
    use chrono::Utc;
    use surrealdb::Value as SurrealValue;

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

    #[test]
    fn new_uses_entity_id_as_record_id() {
        let emb = KnowledgeEntityEmbedding::new(
            "entity-abc",
            "source-1".to_owned(),
            vec![0.1, 0.2],
            "user-1".to_owned(),
            KnowledgeEntity::table_name(),
        );
        assert_eq!(emb.id, "entity-abc");
    }

    #[test]
    fn validate_dimension_rejects_mismatch() {
        let err = KnowledgeEntityEmbedding::validate_dimension(&[0.1, 0.2, 0.3], 2)
            .expect_err("expected dimension mismatch");
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn test_create_and_get_by_entity_id() -> anyhow::Result<()> {
        let db = prepare_knowledge_entity_test_db(3).await?;
        let user_id = "user_ke";
        let entity_key = "entity-1";
        let source_id = "source-ke";

        let embedding_vec = vec![0.11_f32, 0.22, 0.33];
        let entity = build_knowledge_entity_with_id(entity_key, source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), embedding_vec.clone(), 3, &db)
            .await
            .with_context(|| "Failed to store entity with embedding".to_string())?;

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);

        let fetched = KnowledgeEntityEmbedding::get_by_record_id(&db, &entity_rid)
            .await
            .with_context(|| "Failed to get embedding by entity_id".to_string())?
            .ok_or_else(|| anyhow::anyhow!("Expected embedding to exist"))?;

        assert_eq!(fetched.id, entity_key);
        assert_eq!(fetched.user_id, user_id);
        assert_eq!(fetched.source_id, source_id);
        assert_eq!(fetched.entity_id, entity_rid);
        assert_eq!(fetched.embedding, embedding_vec);

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_entity_id() -> anyhow::Result<()> {
        let db = prepare_knowledge_entity_test_db(3).await?;
        let user_id = "user_ke";
        let entity_key = "entity-delete";
        let source_id = "source-del";

        let entity = build_knowledge_entity_with_id(entity_key, source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), vec![0.5_f32, 0.6, 0.7], 3, &db)
            .await
            .with_context(|| "Failed to store entity with embedding".to_string())?;

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);

        let existing = KnowledgeEntityEmbedding::get_by_record_id(&db, &entity_rid)
            .await
            .with_context(|| "Failed to get embedding before delete".to_string())?;
        assert!(existing.is_some());

        KnowledgeEntityEmbedding::delete_by_record_id(&db, &entity_rid)
            .await
            .with_context(|| "Failed to delete by entity_id".to_string())?;

        let after = KnowledgeEntityEmbedding::get_by_record_id(&db, &entity_rid)
            .await
            .with_context(|| "Failed to get embedding after delete".to_string())?;
        assert!(after.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_store_with_embedding_creates_entity_and_embedding() -> anyhow::Result<()> {
        let db = prepare_knowledge_entity_test_db(3).await?;
        let user_id = "user_store";
        let source_id = "source_store";
        let embedding = vec![0.2_f32, 0.3, 0.4];

        let entity = build_knowledge_entity_with_id("entity-store", source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), embedding.clone(), 3, &db)
            .await
            .with_context(|| "Failed to store entity with embedding".to_string())?;

        let stored_entity: Option<KnowledgeEntity> = db
            .get_item(&entity.id)
            .await
            .with_context(|| "Failed to get entity".to_string())?;
        assert!(stored_entity.is_some());

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);
        let stored_embedding = KnowledgeEntityEmbedding::get_by_record_id(&db, &entity_rid)
            .await
            .with_context(|| "Failed to fetch embedding".to_string())?;
        let stored_embedding =
            stored_embedding.ok_or_else(|| anyhow::anyhow!("Expected embedding to exist"))?;
        assert_eq!(stored_embedding.id, entity.id);
        assert_eq!(stored_embedding.user_id, user_id);
        assert_eq!(stored_embedding.source_id, source_id);
        assert_eq!(stored_embedding.entity_id, entity_rid);

        Ok(())
    }

    #[tokio::test]
    async fn test_store_with_embedding_rejects_wrong_dimension() -> anyhow::Result<()> {
        let db = prepare_knowledge_entity_test_db(3).await?;

        let entity = build_knowledge_entity_with_id("entity-dim", "source-dim", "user-dim");
        let result = KnowledgeEntity::store_with_embedding(entity, vec![0.1, 0.2], 3, &db).await;

        assert!(matches!(result, Err(AppError::Validation(_))));

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_source_id() -> anyhow::Result<()> {
        let db = prepare_knowledge_entity_test_db(3).await?;
        let user_id = "user_ke";
        let source_id = "shared-ke";
        let other_source = "other-ke";

        let entity1 = build_knowledge_entity_with_id("entity-s1", source_id, user_id);
        let entity2 = build_knowledge_entity_with_id("entity-s2", source_id, user_id);
        let entity_other = build_knowledge_entity_with_id("entity-other", other_source, user_id);

        KnowledgeEntity::store_with_embedding(entity1.clone(), vec![1.0_f32, 1.1, 1.2], 3, &db)
            .await
            .with_context(|| "Failed to store entity with embedding".to_string())?;
        KnowledgeEntity::store_with_embedding(entity2.clone(), vec![2.0_f32, 2.1, 2.2], 3, &db)
            .await
            .with_context(|| "Failed to store entity with embedding".to_string())?;
        KnowledgeEntity::store_with_embedding(
            entity_other.clone(),
            vec![3.0_f32, 3.1, 3.2],
            3,
            &db,
        )
        .await
        .with_context(|| "Failed to store entity with embedding".to_string())?;

        let entity1_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity1.id);
        let entity2_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity2.id);
        let other_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity_other.id);

        KnowledgeEntityEmbedding::delete_by_source_id(source_id, &db)
            .await
            .with_context(|| "Failed to delete by source_id".to_string())?;

        assert!(
            KnowledgeEntityEmbedding::get_by_record_id(&db, &entity1_rid)
                .await
                .with_context(|| "get entity1 embedding after delete".to_string())?
                .is_none()
        );
        assert!(
            KnowledgeEntityEmbedding::get_by_record_id(&db, &entity2_rid)
                .await
                .with_context(|| "get entity2 embedding after delete".to_string())?
                .is_none()
        );
        assert!(KnowledgeEntityEmbedding::get_by_record_id(&db, &other_rid)
            .await
            .with_context(|| "get other embedding after delete".to_string())?
            .is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_redefine_hnsw_index_updates_dimension() -> anyhow::Result<()> {
        let db = setup_test_db().await?;

        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 16)
            .await
            .with_context(|| "failed to redefine index".to_string())?;

        let mut info_res = db
            .client
            .query("INFO FOR TABLE knowledge_entity_embedding;")
            .await
            .with_context(|| "info query failed".to_string())?;
        let info: SurrealValue = info_res
            .take(0)
            .with_context(|| "failed to take info result".to_string())?;
        let info_json: serde_json::Value = serde_json::to_value(info)
            .with_context(|| "failed to convert info to json".to_string())?;
        let idx_sql = info_json
            .get("Object")
            .and_then(|v| v.get("indexes"))
            .and_then(|v| v.get("Object"))
            .and_then(|v| v.get("idx_embedding_knowledge_entity_embedding"))
            .and_then(|v| v.get("Strand"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        assert!(
            idx_sql.contains("DIMENSION 16"),
            "expected index definition to contain new dimension, got: {idx_sql}"
        );
        assert!(
            idx_sql.contains("DIST COSINE"),
            "expected index definition to use cosine distance, got: {idx_sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_entity_via_record_id() -> anyhow::Result<()> {
        #[derive(Deserialize)]
        struct Row {
            entity_id: KnowledgeEntity,
        }

        let db = prepare_knowledge_entity_test_db(3).await?;
        let user_id = "user_ke";
        let entity_key = "entity-fetch";
        let source_id = "source-fetch";

        let entity = build_knowledge_entity_with_id(entity_key, source_id, user_id);
        KnowledgeEntity::store_with_embedding(entity.clone(), vec![0.7_f32, 0.8, 0.9], 3, &db)
            .await
            .with_context(|| "Failed to store entity with embedding".to_string())?;

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);

        let mut res = db
            .client
            .query(
                "SELECT entity_id FROM knowledge_entity_embedding WHERE entity_id = $id FETCH entity_id;",
            )
            .bind(("id", entity_rid.clone()))
            .await
            .with_context(|| "failed to fetch embedding with FETCH".to_string())?;
        let rows: Vec<Row> = res
            .take(0)
            .with_context(|| "failed to deserialize fetch rows".to_string())?;

        assert_eq!(rows.len(), 1);
        let fetched_entity = &rows
            .first()
            .context("Expected at least one result")?
            .entity_id;
        assert_eq!(fetched_entity.id, entity_key);
        assert_eq!(fetched_entity.name, "Test entity");
        assert_eq!(fetched_entity.user_id, user_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_replaces_existing_embedding_row() -> anyhow::Result<()> {
        let db = prepare_knowledge_entity_test_db(3).await?;

        let user_id = "user-upsert";
        let source_id = "source-upsert";
        let entity = build_knowledge_entity_with_id("entity-upsert", source_id, user_id);

        KnowledgeEntity::store_with_embedding(entity.clone(), vec![1.0_f32, 0.0, 0.0], 3, &db)
            .await
            .with_context(|| "initial store".to_string())?;

        let replacement = KnowledgeEntityEmbedding::new(
            &entity.id,
            source_id.to_owned(),
            vec![0.0, 1.0, 0.0],
            user_id.to_owned(),
            KnowledgeEntity::table_name(),
        );
        db.upsert_item(replacement)
            .await
            .with_context(|| "upsert replacement embedding".to_string())?;

        let entity_rid = RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);
        let rows: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {} WHERE entity_id = $entity_id",
                KnowledgeEntityEmbedding::table_name()
            ))
            .bind(("entity_id", entity_rid))
            .await
            .with_context(|| "count embeddings".to_string())?
            .take(0)
            .with_context(|| "take embeddings".to_string())?;

        assert_eq!(rows.len(), 1);
        let row = rows.first().expect("expected one embedding row");
        assert_eq!(row.id, entity.id);
        assert_eq!(row.embedding, vec![0.0, 1.0, 0.0]);

        Ok(())
    }
}
