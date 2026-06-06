#![allow(clippy::missing_docs_in_private_items, clippy::module_name_repetitions)]
use std::collections::HashMap;
use std::fmt::Write;

use crate::{
    error::AppError, storage::db::SurrealDbClient, storage::indexes::hnsw_index_overwrite_sql,
    storage::types::knowledge_entity_embedding::KnowledgeEntityEmbedding,
    storage::types::system_settings::SystemSettings, stored_object,
    utils::embedding::{EmbeddingProvider, RE_EMBED_BATCH_SIZE},
};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum KnowledgeEntityType {
    Idea,
    Project,
    Document,
    Page,
    TextSnippet,
    // Add more types as needed
}
impl KnowledgeEntityType {
    #[must_use]
    pub fn variants() -> &'static [&'static str] {
        &["Idea", "Project", "Document", "Page", "TextSnippet"]
    }
}

impl From<String> for KnowledgeEntityType {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "idea" => KnowledgeEntityType::Idea,
            "project" => KnowledgeEntityType::Project,
            "page" => KnowledgeEntityType::Page,
            "textsnippet" => KnowledgeEntityType::TextSnippet,
            _ => KnowledgeEntityType::Document, // Default case
        }
    }
}

/// Search result including hydrated entity.
#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeEntitySearchResult {
    pub entity: KnowledgeEntity,
    pub score: f32,
}

stored_object!(KnowledgeEntity, "knowledge_entity", {
    source_id: String,
    name: String,
    description: String,
    entity_type: KnowledgeEntityType,
    metadata: Option<serde_json::Value>,
    user_id: String
});

impl KnowledgeEntity {
    #[must_use]
    pub fn new(
        source_id: String,
        name: String,
        description: String,
        entity_type: KnowledgeEntityType,
        metadata: Option<serde_json::Value>,
        user_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id,
            name,
            description,
            entity_type,
            metadata,
            user_id,
        }
    }

    /// Full-text search over knowledge entities using the BM25 FTS index.
    pub async fn fts_search(
        take: usize,
        terms: &str,
        db: &SurrealDbClient,
        user_id: &str,
    ) -> Result<Vec<KnowledgeEntitySearchResult>, AppError> {
        #[derive(Deserialize)]
        struct Row {
            #[serde(deserialize_with = "deserialize_flexible_id")]
            id: String,
            #[serde(deserialize_with = "deserialize_datetime")]
            created_at: DateTime<Utc>,
            #[serde(deserialize_with = "deserialize_datetime")]
            updated_at: DateTime<Utc>,
            source_id: String,
            name: String,
            description: String,
            entity_type: KnowledgeEntityType,
            #[serde(default)]
            metadata: Option<serde_json::Value>,
            user_id: String,
            score: f32,
        }

        let limit = i64::try_from(take).unwrap_or(i64::MAX);

        let sql = r#"
            SELECT
                id,
                created_at,
                updated_at,
                source_id,
                name,
                description,
                entity_type,
                metadata,
                user_id,
                (
                    IF search::score(0) != NONE THEN search::score(0) ELSE 0 END +
                    IF search::score(1) != NONE THEN search::score(1) ELSE 0 END
                ) AS score
            FROM knowledge_entity
            WHERE
                (
                    name @0@ $terms OR
                    description @1@ $terms
                )
                AND user_id = $user_id
            ORDER BY score DESC
            LIMIT $limit;
        "#;

        let rows: Vec<Row> = db
            .client
            .query(sql)
            .bind(("terms", terms.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("limit", limit))
            .await?
            .take(0)?;

        Ok(rows
            .into_iter()
            .map(|row| KnowledgeEntitySearchResult {
                entity: KnowledgeEntity {
                    id: row.id,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    source_id: row.source_id,
                    name: row.name,
                    description: row.description,
                    entity_type: row.entity_type,
                    metadata: row.metadata,
                    user_id: row.user_id,
                },
                score: row.score,
            })
            .collect())
    }

    /// Fetch all knowledge entities owned by any of the provided source ids for a user.
    ///
    /// Used by retrieval to resolve the entities that own a set of retrieved chunks.
    pub async fn find_by_source_ids(
        db: &SurrealDbClient,
        source_ids: &[String],
        user_id: &str,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        if source_ids.is_empty() {
            return Ok(Vec::new());
        }

        let entities: Vec<KnowledgeEntity> = db
            .client
            .query(
                "SELECT * FROM type::table($table) \
                 WHERE source_id IN $sources AND user_id = $user_id",
            )
            .bind(("table", Self::table_name()))
            .bind(("sources", source_ids.to_vec()))
            .bind(("user_id", user_id.to_owned()))
            .await
            .map_err(AppError::from)?
            .take(0)
            .map_err(AppError::from)?;

        Ok(entities)
    }

    pub async fn delete_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        // Delete embeddings first, while we can still look them up via the entity's source_id
        KnowledgeEntityEmbedding::delete_by_source_id(source_id, db_client).await?;

        db_client
            .client
            .query("DELETE FROM type::table($table) WHERE source_id = $source_id")
            .bind(("table", Self::table_name()))
            .bind(("source_id", source_id.to_owned()))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        Ok(())
    }

    /// Atomically store a knowledge entity and its embedding.
    /// Writes the entity to `knowledge_entity` and the embedding to `knowledge_entity_embedding`.
    pub async fn store_with_embedding(
        entity: KnowledgeEntity,
        embedding: Vec<f32>,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let settings = SystemSettings::get_current(db).await?;
        KnowledgeEntityEmbedding::validate_dimension(
            &embedding,
            settings.embedding_dimensions as usize,
        )?;

        let entity_id = entity.id.clone();
        let emb = KnowledgeEntityEmbedding::new(
            &entity_id,
            entity.source_id.clone(),
            embedding,
            entity.user_id.clone(),
        );

        let query = format!(
            "
            BEGIN TRANSACTION;
              CREATE type::thing('{entity_table}', $entity_id) CONTENT $entity;
              UPSERT type::thing('{emb_table}', $entity_id) CONTENT $emb;
            COMMIT TRANSACTION;
            ",
            entity_table = Self::table_name(),
            emb_table = KnowledgeEntityEmbedding::table_name(),
        );

        db.client
            .query(query)
            .bind(("entity_id", entity_id))
            .bind(("entity", entity))
            .bind(("emb", emb))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        Ok(())
    }

    /// Vector search over knowledge entities using the embedding table, fetching full entity rows and scores.
    pub async fn vector_search(
        take: usize,
        query_embedding: &[f32],
        db: &SurrealDbClient,
        user_id: &str,
    ) -> Result<Vec<KnowledgeEntitySearchResult>, AppError> {
        #[derive(Deserialize)]
        struct Row {
            entity_id: Option<KnowledgeEntity>,
            score: f32,
        }

        let sql = format!(
            r#"
            SELECT
                entity_id,
                vector::similarity::cosine(embedding, $embedding) AS score
            FROM {emb_table}
            WHERE user_id = $user_id
              AND embedding <|{take},100|> $embedding
            ORDER BY score DESC
            LIMIT {take}
            FETCH entity_id;
            "#,
            emb_table = KnowledgeEntityEmbedding::table_name(),
            take = take
        );

        let mut response = db
            .query(&sql)
            .bind(("embedding", query_embedding.to_vec()))
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(AppError::from)?;

        response = response.check().map_err(AppError::from)?;

        let rows: Vec<Row> = response.take::<Vec<Row>>(0).map_err(AppError::from)?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                r.entity_id.map(|entity| KnowledgeEntitySearchResult {
                    entity,
                    score: r.score,
                })
            })
            .collect())
    }

    pub async fn patch(
        id: &str,
        name: &str,
        description: &str,
        entity_type: &KnowledgeEntityType,
        db_client: &SurrealDbClient,
        embedding_provider: &EmbeddingProvider,
    ) -> Result<(), AppError> {
        let embedding_input =
            format!("name: {name}, description: {description}, type: {entity_type:?}",);
        let embedding = embedding_provider.embed(&embedding_input).await?;

        let entity: KnowledgeEntity = db_client
            .get_item(id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::NotFound(format!("entity {id} not found")))?;

        let settings = SystemSettings::get_current(db_client).await?;
        KnowledgeEntityEmbedding::validate_dimension(
            &embedding,
            settings.embedding_dimensions as usize,
        )?;

        let emb = KnowledgeEntityEmbedding::new(id, entity.source_id, embedding, entity.user_id);

        let now = Utc::now();

        db_client
            .client
            .query(
                "BEGIN TRANSACTION;
                 UPDATE type::thing($table, $id)
                 SET name = $name,
                     description = $description,
                     updated_at = $updated_at,
                     entity_type = $entity_type;
                 UPSERT type::thing($emb_table, $id) CONTENT $emb;
                 COMMIT TRANSACTION;",
            )
            .bind(("table", Self::table_name()))
            .bind(("emb_table", KnowledgeEntityEmbedding::table_name()))
            .bind(("id", id.to_string()))
            .bind(("name", name.to_string()))
            .bind(("updated_at", surrealdb::Datetime::from(now)))
            .bind(("entity_type", entity_type.to_owned()))
            .bind(("emb", emb))
            .bind(("description", description.to_string()))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        Ok(())
    }

    /// Re-creates embeddings for all knowledge entities using an `EmbeddingProvider`.
    ///
    /// This is a costly operation that should be run in the background. It follows the same
    /// pattern as the text chunk update:
    /// 1. Re-defines the vector index with the new dimensions.
    /// 2. Fetches all existing entities.
    /// 3. Sequentially regenerates the embedding for each and updates the record.
    #[allow(clippy::too_many_lines)]
    pub async fn update_all_embeddings(
        db: &SurrealDbClient,
        provider: &EmbeddingProvider,
    ) -> Result<(), AppError> {
        let new_dimensions = provider.dimension();
        info!(
            dimensions = new_dimensions,
            backend = provider.backend_label(),
            "Starting re-embedding process for all knowledge entities"
        );

        // Fetch all entities first
        let all_entities: Vec<KnowledgeEntity> = db.select(Self::table_name()).await?;
        let total_entities = all_entities.len();
        if total_entities == 0 {
            info!("No knowledge entities to update. Just updating the index.");
            KnowledgeEntityEmbedding::redefine_hnsw_index(db, new_dimensions).await?;
            return Ok(());
        }
        info!(entities = total_entities, "Found entities to process");

        // Generate all new embeddings in memory, batching to amortise lock/dispatch overhead
        // while keeping memory bounded and preserving progress logging.
        let mut new_embeddings: HashMap<String, (Vec<f32>, String, String)> =
            HashMap::with_capacity(total_entities);
        info!("Generating new embeddings for all entities...");

        let mut processed = 0usize;
        for batch in all_entities.chunks(RE_EMBED_BATCH_SIZE) {
            let inputs: Vec<String> = batch
                .iter()
                .map(|entity| {
                    format!(
                        "name: {}, description: {}, type: {:?}",
                        entity.name, entity.description, entity.entity_type
                    )
                })
                .collect();
            let embeddings = provider.embed_batch(&inputs).await?;
            if embeddings.len() != batch.len() {
                return Err(AppError::internal(format!(
                    "embedding batch returned {} vectors for {} entities",
                    embeddings.len(),
                    batch.len()
                )));
            }

            for (entity, embedding) in batch.iter().zip(embeddings) {
                // Safety check: ensure the generated embedding has the correct dimension.
                if embedding.len() != new_dimensions {
                    let err_msg = format!(
                        "CRITICAL: Generated embedding for entity {} has incorrect dimension ({}). Expected {}. Aborting.",
                        entity.id, embedding.len(), new_dimensions
                    );
                    error!("{err_msg}");
                    return Err(AppError::internal(err_msg));
                }
                new_embeddings.insert(
                    entity.id.clone(),
                    (embedding, entity.user_id.clone(), entity.source_id.clone()),
                );
            }

            processed = processed.saturating_add(batch.len());
            info!(
                progress = processed,
                total = total_entities,
                "Re-embedding progress"
            );
        }
        info!("Successfully generated all new embeddings.");

        // Clear existing embeddings and index first to prevent SurrealDB panics and dimension conflicts.
        info!("Removing old index and clearing embeddings...");

        // Explicitly remove the index first. This prevents background HNSW maintenance from crashing
        // when we delete/replace data, dealing with a known SurrealDB panic.
        db.client
            .query(format!(
                "REMOVE INDEX idx_embedding_knowledge_entity_embedding ON TABLE {};",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        db.client
            .query(format!(
                "DELETE FROM {};",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        // Perform DB updates in a single transaction
        info!("Applying embedding updates in a transaction...");
        let mut transaction_query = String::from("BEGIN TRANSACTION;");

        for (id, (embedding, user_id, source_id)) in new_embeddings {
            let embedding = serde_json::to_string(&embedding)
                .map_err(|e| AppError::internal(format!("embedding serialization failed: {e}")))?;
            write!(
                transaction_query,
                "CREATE type::thing('knowledge_entity_embedding', '{id}') SET \
                    entity_id = type::thing('knowledge_entity', '{id}'), \
                    embedding = {embedding}, \
                    user_id = '{user_id}', \
                    source_id = '{source_id}', \
                    created_at = time::now(), \
                    updated_at = time::now();",
            )
            .map_err(AppError::internal)?;
        }

        write!(
            transaction_query,
            "{}",
            hnsw_index_overwrite_sql(
                "idx_embedding_knowledge_entity_embedding",
                KnowledgeEntityEmbedding::table_name(),
                new_dimensions,
            )
        )
        .map_err(AppError::internal)?;

        transaction_query.push_str("COMMIT TRANSACTION;");

        // Execute the entire atomic operation
        db.client
            .query(transaction_query)
            .await
            .map_err(AppError::from)?
            .check()
            .map_err(AppError::from)?;

        info!("Re-embedding process for knowledge entities completed successfully.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use super::*;
    use crate::storage::indexes::rebuild;
    use crate::storage::types::knowledge_entity_embedding::KnowledgeEntityEmbedding;
    use crate::test_utils::configure_embedding_dimension;
    use anyhow::{self, Context};
    use uuid::Uuid;

    async fn ensure_entity_fts_indexes(db: &SurrealDbClient) -> anyhow::Result<()> {
        let snowball_sql = r#"
            DEFINE ANALYZER IF NOT EXISTS app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii, snowball(english);
            DEFINE INDEX IF NOT EXISTS knowledge_entity_fts_name_idx ON TABLE knowledge_entity FIELDS name SEARCH ANALYZER app_en_fts_analyzer BM25;
            DEFINE INDEX IF NOT EXISTS knowledge_entity_fts_description_idx ON TABLE knowledge_entity FIELDS description SEARCH ANALYZER app_en_fts_analyzer BM25;
        "#;

        if let Err(err) = db.client.query(snowball_sql).await {
            let fallback_sql = r#"
                DEFINE ANALYZER OVERWRITE app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii;
                DEFINE INDEX IF NOT EXISTS knowledge_entity_fts_name_idx ON TABLE knowledge_entity FIELDS name SEARCH ANALYZER app_en_fts_analyzer BM25;
                DEFINE INDEX IF NOT EXISTS knowledge_entity_fts_description_idx ON TABLE knowledge_entity FIELDS description SEARCH ANALYZER app_en_fts_analyzer BM25;
            "#;

            db.client
                .query(fallback_sql)
                .await
                .with_context(|| format!("define entity fts index fallback: {err}"))?;
        }
        Ok(())
    }
    use serde_json::json;

    #[tokio::test]
    async fn test_knowledge_entity_creation() -> anyhow::Result<()> {
        let source_id = "source123".to_string();
        let name = "Test Entity".to_string();
        let description = "Test Description".to_string();
        let entity_type = KnowledgeEntityType::Document;
        let metadata = Some(json!({"key": "value"}));
        let user_id = "user123".to_string();

        let entity = KnowledgeEntity::new(
            source_id.clone(),
            name.clone(),
            description.clone(),
            entity_type,
            metadata.clone(),
            user_id.clone(),
        );

        assert_eq!(entity.source_id, source_id);
        assert_eq!(entity.name, name);
        assert_eq!(entity.description, description);
        assert_eq!(entity.entity_type, entity_type);
        assert_eq!(entity.metadata, metadata);
        assert_eq!(entity.user_id, user_id);
        assert!(!entity.id.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_knowledge_entity_type_from_string() -> anyhow::Result<()> {
        assert_eq!(
            KnowledgeEntityType::from("idea".to_string()),
            KnowledgeEntityType::Idea
        );
        assert_eq!(
            KnowledgeEntityType::from("Idea".to_string()),
            KnowledgeEntityType::Idea
        );
        assert_eq!(
            KnowledgeEntityType::from("IDEA".to_string()),
            KnowledgeEntityType::Idea
        );

        assert_eq!(
            KnowledgeEntityType::from("project".to_string()),
            KnowledgeEntityType::Project
        );
        assert_eq!(
            KnowledgeEntityType::from("document".to_string()),
            KnowledgeEntityType::Document
        );
        assert_eq!(
            KnowledgeEntityType::from("page".to_string()),
            KnowledgeEntityType::Page
        );
        assert_eq!(
            KnowledgeEntityType::from("textsnippet".to_string()),
            KnowledgeEntityType::TextSnippet
        );

        assert_eq!(
            KnowledgeEntityType::from("unknown".to_string()),
            KnowledgeEntityType::Document
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_knowledge_entity_variants() -> anyhow::Result<()> {
        let variants = KnowledgeEntityType::variants();
        assert_eq!(variants.len(), 5);
        assert!(variants.contains(&"Idea"));
        assert!(variants.contains(&"Project"));
        assert!(variants.contains(&"Document"));
        assert!(variants.contains(&"Page"));
        assert!(variants.contains(&"TextSnippet"));

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_source_id() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;

        configure_embedding_dimension(&db, 5).await?;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .with_context(|| "Failed to redefine index length".to_string())?;

        let source_id = "source123".to_string();
        let entity_type = KnowledgeEntityType::Document;
        let user_id = "user123".to_string();

        let entity1 = KnowledgeEntity::new(
            source_id.clone(),
            "Entity 1".to_string(),
            "Description 1".to_string(),
            entity_type,
            None,
            user_id.clone(),
        );

        let entity2 = KnowledgeEntity::new(
            source_id.clone(),
            "Entity 2".to_string(),
            "Description 2".to_string(),
            entity_type,
            None,
            user_id.clone(),
        );

        let different_source_id = "different_source".to_string();
        let different_entity = KnowledgeEntity::new(
            different_source_id.clone(),
            "Different Entity".to_string(),
            "Different Description".to_string(),
            entity_type,
            None,
            user_id.clone(),
        );

        let emb = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        KnowledgeEntity::store_with_embedding(entity1.clone(), emb.clone(), &db)
            .await
            .with_context(|| "Failed to store entity 1".to_string())?;
        KnowledgeEntity::store_with_embedding(entity2.clone(), emb.clone(), &db)
            .await
            .with_context(|| "Failed to store entity 2".to_string())?;
        KnowledgeEntity::store_with_embedding(different_entity.clone(), emb.clone(), &db)
            .await
            .with_context(|| "Failed to store different entity".to_string())?;

        KnowledgeEntity::delete_by_source_id(&source_id, &db)
            .await
            .with_context(|| "Failed to delete entities by source_id".to_string())?;

        let query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            KnowledgeEntity::table_name(),
            source_id
        );
        let remaining: Vec<KnowledgeEntity> = db
            .client
            .query(query)
            .await
            .with_context(|| "Query failed".to_string())?
            .take(0)
            .with_context(|| "Failed to get query results".to_string())?;
        assert!(
            remaining.is_empty(),
            "All entities with the source_id should be deleted"
        );

        let different_query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            KnowledgeEntity::table_name(),
            different_source_id
        );
        let different_remaining: Vec<KnowledgeEntity> = db
            .client
            .query(different_query)
            .await
            .with_context(|| "Query failed".to_string())?
            .take(0)
            .with_context(|| "Failed to get query results".to_string())?;
        assert_eq!(
            different_remaining.len(),
            1,
            "Entity with different source_id should still exist"
        );
        assert_eq!(
            different_remaining
                .first()
                .context("Expected entity to exist")?
                .id,
            different_entity.id
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_by_source_id_resists_query_injection() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        configure_embedding_dimension(&db, 3)
            .await
            .expect("configure dim");
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("Failed to redefine index length");

        let user_id = "user123".to_string();

        let entity1 = KnowledgeEntity::new(
            "safe_source".to_string(),
            "Entity 1".to_string(),
            "Description 1".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.clone(),
        );

        let entity2 = KnowledgeEntity::new(
            "other_source".to_string(),
            "Entity 2".to_string(),
            "Description 2".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id,
        );

        KnowledgeEntity::store_with_embedding(entity1, vec![0.1, 0.2, 0.3], &db)
            .await
            .expect("store entity1");
        KnowledgeEntity::store_with_embedding(entity2, vec![0.3, 0.2, 0.1], &db)
            .await
            .expect("store entity2");

        let malicious_source = "safe_source' OR 1=1 --";
        KnowledgeEntity::delete_by_source_id(malicious_source, &db)
            .await
            .expect("delete call should succeed");

        let remaining: Vec<KnowledgeEntity> = db
            .client
            .query("SELECT * FROM type::table($table)")
            .bind(("table", KnowledgeEntity::table_name()))
            .await
            .expect("query failed")
            .take(0)
            .expect("take failed");

        assert_eq!(
            remaining.len(),
            2,
            "malicious input must not delete unrelated entities"
        );
    }

    #[tokio::test]
    async fn test_vector_search_returns_empty_when_no_embeddings() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .expect("Failed to redefine index length");

        let results = KnowledgeEntity::vector_search(5, &[0.1, 0.2, 0.3], &db, "user")
            .await
            .expect("vector search");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_vector_search_single_result() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;

        configure_embedding_dimension(&db, 3).await?;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "Failed to redefine index length".to_string())?;

        let user_id = "user".to_string();
        let source_id = "src".to_string();
        let entity = KnowledgeEntity::new(
            source_id.clone(),
            "hello".to_string(),
            "world".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.clone(),
        );

        KnowledgeEntity::store_with_embedding(entity.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .with_context(|| "store entity with embedding".to_string())?;

        let stored_entity: Option<KnowledgeEntity> = db
            .get_item(&entity.id)
            .await
            .with_context(|| "Failed to get entity".to_string())?;
        assert!(stored_entity.is_some());

        let stored_embeddings: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {}",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .with_context(|| "query embeddings".to_string())?
            .take(0)
            .with_context(|| "take embeddings".to_string())?;
        assert_eq!(stored_embeddings.len(), 1);

        let rid = surrealdb::RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);
        let fetched_emb = KnowledgeEntityEmbedding::get_by_entity_id(&rid, &db)
            .await
            .with_context(|| "fetch embedding".to_string())?;
        assert!(fetched_emb.is_some());

        let results = KnowledgeEntity::vector_search(3, &[0.1, 0.2, 0.3], &db, &user_id)
            .await
            .with_context(|| "vector search".to_string())?;

        assert_eq!(results.len(), 1);
        let res = results.first().context("Expected at least one result")?;
        assert_eq!(res.entity.id, entity.id);
        assert_eq!(res.entity.source_id, source_id);
        assert_eq!(res.entity.name, "hello");

        Ok(())
    }

    #[tokio::test]
    async fn test_vector_search_orders_by_similarity() -> anyhow::Result<()> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;

        configure_embedding_dimension(&db, 3).await?;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "Failed to redefine index length".to_string())?;

        let user_id = "user".to_string();
        let e1 = KnowledgeEntity::new(
            "s1".to_string(),
            "entity one".to_string(),
            "desc".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.clone(),
        );
        let e2 = KnowledgeEntity::new(
            "s2".to_string(),
            "entity two".to_string(),
            "desc".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.clone(),
        );

        KnowledgeEntity::store_with_embedding(e1.clone(), vec![1.0, 0.0, 0.0], &db)
            .await
            .with_context(|| "store e1".to_string())?;
        KnowledgeEntity::store_with_embedding(e2.clone(), vec![0.0, 1.0, 0.0], &db)
            .await
            .with_context(|| "store e2".to_string())?;

        let stored_e1: Option<KnowledgeEntity> = db
            .get_item(&e1.id)
            .await
            .with_context(|| "Failed to get entity".to_string())?;
        let stored_e2: Option<KnowledgeEntity> = db
            .get_item(&e2.id)
            .await
            .with_context(|| "Failed to get entity".to_string())?;
        assert!(stored_e1.is_some() && stored_e2.is_some());

        let stored_embeddings: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {}",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .with_context(|| "query embeddings".to_string())?
            .take(0)
            .with_context(|| "take embeddings".to_string())?;
        assert_eq!(stored_embeddings.len(), 2);

        let rid_e1 = surrealdb::RecordId::from_table_key(KnowledgeEntity::table_name(), &e1.id);
        let rid_e2 = surrealdb::RecordId::from_table_key(KnowledgeEntity::table_name(), &e2.id);
        assert!(KnowledgeEntityEmbedding::get_by_entity_id(&rid_e1, &db)
            .await
            .with_context(|| "get embedding e1".to_string())?
            .is_some());
        assert!(KnowledgeEntityEmbedding::get_by_entity_id(&rid_e2, &db)
            .await
            .with_context(|| "get embedding e2".to_string())?
            .is_some());

        let results = KnowledgeEntity::vector_search(2, &[0.0, 1.0, 0.0], &db, &user_id)
            .await
            .with_context(|| "vector search".to_string())?;

        assert_eq!(results.len(), 2);
        assert_eq!(
            results
                .first()
                .context("Expected at least one result")?
                .entity
                .id,
            e2.id
        );
        assert_eq!(
            results
                .get(1)
                .context("Expected at least two results")?
                .entity
                .id,
            e1.id
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_vector_search_with_orphaned_embedding() -> anyhow::Result<()> {
        let namespace = "test_ns_orphan";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "Failed to apply migrations".to_string())?;

        configure_embedding_dimension(&db, 3).await?;
        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 3)
            .await
            .with_context(|| "Failed to redefine index length".to_string())?;

        let user_id = "user".to_string();
        let source_id = "src".to_string();
        let entity = KnowledgeEntity::new(
            source_id.clone(),
            "orphan".to_string(),
            "orphan desc".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.clone(),
        );

        KnowledgeEntity::store_with_embedding(entity.clone(), vec![0.1, 0.2, 0.3], &db)
            .await
            .with_context(|| "store entity with embedding".to_string())?;

        let query = format!(
            "DELETE type::thing('knowledge_entity', '{id}')",
            id = entity.id
        );
        db.client
            .query(query)
            .await
            .with_context(|| "delete entity".to_string())?;

        let results = KnowledgeEntity::vector_search(3, &[0.1, 0.2, 0.3], &db, &user_id)
            .await
            .with_context(|| "search should succeed even with orphans".to_string())?;

        assert!(
            results.is_empty(),
            "Should return empty result for orphan, got: {results:?}",
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_fts_search_returns_empty_when_no_entities() -> anyhow::Result<()> {
        let namespace = "fts_entity_ns_empty";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        ensure_entity_fts_indexes(&db).await?;
        rebuild(&db)
            .await
            .with_context(|| "rebuild indexes".to_string())?;

        let results = KnowledgeEntity::fts_search(5, "hello", &db, "user")
            .await
            .with_context(|| "fts search".to_string())?;

        assert!(results.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_fts_search_single_result() -> anyhow::Result<()> {
        let namespace = "fts_entity_ns_single";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        ensure_entity_fts_indexes(&db).await?;

        let user_id = "fts_user";
        let entity = KnowledgeEntity::new(
            "fts_src".to_string(),
            "cucumber".to_string(),
            "cucumbers are best".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.to_string(),
        );
        db.store_item(entity.clone())
            .await
            .with_context(|| "store entity".to_string())?;
        rebuild(&db)
            .await
            .with_context(|| "rebuild indexes".to_string())?;

        let results = KnowledgeEntity::fts_search(3, "cucumber", &db, user_id)
            .await
            .with_context(|| "fts search".to_string())?;

        assert_eq!(results.len(), 1);
        let r0 = results.first().context("expected first result")?;
        assert_eq!(r0.entity.id, entity.id);
        assert!(r0.score.is_finite(), "expected a finite FTS score");
        Ok(())
    }

    #[tokio::test]
    async fn test_fts_search_orders_by_score_and_filters_user() -> anyhow::Result<()> {
        let namespace = "fts_entity_ns_order";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .with_context(|| "Failed to start in-memory surrealdb".to_string())?;
        db.apply_migrations()
            .await
            .with_context(|| "migrations".to_string())?;
        ensure_entity_fts_indexes(&db).await?;

        let user_id = "fts_user_order";
        let high_score_entity = KnowledgeEntity::new(
            "src1".to_string(),
            "apple apple apple pie".to_string(),
            "dessert recipe".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.to_string(),
        );
        let low_score_entity = KnowledgeEntity::new(
            "src2".to_string(),
            "apple tart".to_string(),
            "light dessert".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.to_string(),
        );
        let other_user_entity = KnowledgeEntity::new(
            "src3".to_string(),
            "apple orchard".to_string(),
            "farming guide".to_string(),
            KnowledgeEntityType::Document,
            None,
            "other_user".to_string(),
        );

        db.store_item(high_score_entity.clone())
            .await
            .with_context(|| "store high score entity".to_string())?;
        db.store_item(low_score_entity.clone())
            .await
            .with_context(|| "store low score entity".to_string())?;
        db.store_item(other_user_entity)
            .await
            .with_context(|| "store other user entity".to_string())?;
        rebuild(&db)
            .await
            .with_context(|| "rebuild indexes".to_string())?;

        let results = KnowledgeEntity::fts_search(3, "apple", &db, user_id)
            .await
            .with_context(|| "fts search".to_string())?;

        assert_eq!(results.len(), 2);
        let ids: Vec<_> = results.iter().map(|r| r.entity.id.as_str()).collect();
        assert!(
            ids.contains(&high_score_entity.id.as_str())
                && ids.contains(&low_score_entity.id.as_str()),
            "expected only the two entities for the same user"
        );
        let r0 = results.first().context("expected first result")?;
        let r1 = results.get(1).context("expected second result")?;
        assert!(r0.score >= r1.score);
        Ok(())
    }
}
