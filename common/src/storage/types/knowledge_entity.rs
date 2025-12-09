#![allow(
    clippy::missing_docs_in_private_items,
    clippy::module_name_repetitions,
    clippy::match_same_arms,
    clippy::format_push_string,
    clippy::uninlined_format_args,
    clippy::explicit_iter_loop,
    clippy::items_after_statements,
    clippy::get_first,
    clippy::redundant_closure_for_method_calls
)]
use std::collections::HashMap;

use crate::{
    error::AppError, storage::db::SurrealDbClient,
    storage::types::knowledge_entity_embedding::KnowledgeEntityEmbedding, stored_object,
    utils::embedding::generate_embedding,
};
use async_openai::{config::OpenAIConfig, Client};
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum KnowledgeEntityType {
    Idea,
    Project,
    Document,
    Page,
    TextSnippet,
    // Add more types as needed
}
impl KnowledgeEntityType {
    pub fn variants() -> &'static [&'static str] {
        &["Idea", "Project", "Document", "Page", "TextSnippet"]
    }
}

impl From<String> for KnowledgeEntityType {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "idea" => KnowledgeEntityType::Idea,
            "project" => KnowledgeEntityType::Project,
            "document" => KnowledgeEntityType::Document,
            "page" => KnowledgeEntityType::Page,
            "textsnippet" => KnowledgeEntityType::TextSnippet,
            _ => KnowledgeEntityType::Document, // Default case
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct KnowledgeEntitySearchResult {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: String,
    #[serde(
        serialize_with = "serialize_datetime",
        deserialize_with = "deserialize_datetime",
        default
    )]
    pub created_at: DateTime<Utc>,
    #[serde(
        serialize_with = "serialize_datetime",
        deserialize_with = "deserialize_datetime",
        default
    )]
    pub updated_at: DateTime<Utc>,

    pub source_id: String,
    pub name: String,
    pub description: String,
    pub entity_type: KnowledgeEntityType,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    pub user_id: String,

    pub score: f32,
    #[serde(default)]
    pub highlighted_name: Option<String>,
    #[serde(default)]
    pub highlighted_description: Option<String>,
}

stored_object!(KnowledgeEntity, "knowledge_entity", {
    source_id: String,
    name: String,
    description: String,
    entity_type: KnowledgeEntityType,
    metadata: Option<serde_json::Value>,
    user_id: String
});

/// Vector search result including hydrated entity.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct KnowledgeEntityVectorResult {
    pub entity: KnowledgeEntity,
    pub score: f32,
}

impl KnowledgeEntity {
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

    pub async fn search(
        db: &SurrealDbClient,
        search_terms: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntitySearchResult>, AppError> {
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
                search::highlight('<b>', '</b>', 0) AS highlighted_name,
                search::highlight('<b>', '</b>', 1) AS highlighted_description,
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

        Ok(db
            .client
            .query(sql)
            .bind(("terms", search_terms.to_owned()))
            .bind(("user_id", user_id.to_owned()))
            .bind(("limit", limit))
            .await?
            .take(0)?)
    }

    pub async fn delete_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE {} WHERE source_id = '{}'",
            Self::table_name(),
            source_id
        );
        db_client.query(query).await?;

        Ok(())
    }

    /// Atomically store a knowledge entity and its embedding.
    /// Writes the entity to `knowledge_entity` and the embedding to `knowledge_entity_embedding`.
    pub async fn store_with_embedding(
        entity: KnowledgeEntity,
        embedding: Vec<f32>,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let emb = KnowledgeEntityEmbedding::new(&entity.id, embedding, entity.user_id.clone());

        let query = format!(
            "
            BEGIN TRANSACTION;
              CREATE type::thing('{entity_table}', $entity_id) CONTENT $entity;
              CREATE type::thing('{emb_table}', $emb_id) CONTENT $emb;
            COMMIT TRANSACTION;
            ",
            entity_table = Self::table_name(),
            emb_table = KnowledgeEntityEmbedding::table_name(),
        );

        db.client
            .query(query)
            .bind(("entity_id", entity.id.clone()))
            .bind(("entity", entity))
            .bind(("emb_id", emb.id.clone()))
            .bind(("emb", emb))
            .await
            .map_err(AppError::Database)?
            .check()
            .map_err(AppError::Database)?;

        Ok(())
    }

    /// Vector search over knowledge entities using the embedding table, fetching full entity rows and scores.
    pub async fn vector_search(
        take: usize,
        query_embedding: Vec<f32>,
        db: &SurrealDbClient,
        user_id: &str,
    ) -> Result<Vec<KnowledgeEntityVectorResult>, AppError> {
        #[derive(Deserialize)]
        struct Row {
            entity_id: KnowledgeEntity,
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
            .bind(("embedding", query_embedding))
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::InternalError(format!("Surreal query failed: {e}")))?;

        response = response.check().map_err(AppError::Database)?;

        let rows: Vec<Row> = response.take::<Vec<Row>>(0).map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| KnowledgeEntityVectorResult {
                entity: r.entity_id,
                score: r.score,
            })
            .collect())
    }

    pub async fn patch(
        id: &str,
        name: &str,
        description: &str,
        entity_type: &KnowledgeEntityType,
        db_client: &SurrealDbClient,
        ai_client: &Client<OpenAIConfig>,
    ) -> Result<(), AppError> {
        let embedding_input = format!(
            "name: {}, description: {}, type: {:?}",
            name, description, entity_type
        );
        let embedding = generate_embedding(ai_client, &embedding_input, db_client).await?;
        let user_id = Self::get_user_id_by_id(id, db_client).await?;
        let emb = KnowledgeEntityEmbedding::new(id, embedding, user_id);

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
                 UPSERT type::thing($emb_table, $emb_id) CONTENT $emb;
                 COMMIT TRANSACTION;",
            )
            .bind(("table", Self::table_name()))
            .bind(("emb_table", KnowledgeEntityEmbedding::table_name()))
            .bind(("id", id.to_string()))
            .bind(("name", name.to_string()))
            .bind(("updated_at", surrealdb::Datetime::from(now)))
            .bind(("entity_type", entity_type.to_owned()))
            .bind(("emb_id", emb.id.clone()))
            .bind(("emb", emb))
            .bind(("description", description.to_string()))
            .await?;

        Ok(())
    }

    async fn get_user_id_by_id(id: &str, db_client: &SurrealDbClient) -> Result<String, AppError> {
        let mut response = db_client
            .client
            .query("SELECT user_id FROM type::thing($table, $id) LIMIT 1")
            .bind(("table", Self::table_name()))
            .bind(("id", id.to_string()))
            .await
            .map_err(AppError::Database)?;
        #[derive(Deserialize)]
        struct Row {
            user_id: String,
        }
        let rows: Vec<Row> = response.take(0).map_err(AppError::Database)?;
        rows.get(0)
            .map(|r| r.user_id.clone())
            .ok_or_else(|| AppError::InternalError("user not found for entity".to_string()))
    }

    /// Re-creates embeddings for all knowledge entities in the database.
    ///
    /// This is a costly operation that should be run in the background. It follows the same
    /// pattern as the text chunk update:
    /// 1. Re-defines the vector index with the new dimensions.
    /// 2. Fetches all existing entities.
    /// 3. Sequentially regenerates the embedding for each and updates the record.
    pub async fn update_all_embeddings(
        db: &SurrealDbClient,
        openai_client: &Client<OpenAIConfig>,
        new_model: &str,
        new_dimensions: u32,
    ) -> Result<(), AppError> {
        info!(
            "Starting re-embedding process for all knowledge entities. New dimensions: {}",
            new_dimensions
        );

        // Fetch all entities first
        let all_entities: Vec<KnowledgeEntity> = db.select(Self::table_name()).await?;
        let total_entities = all_entities.len();
        if total_entities == 0 {
            info!("No knowledge entities to update. Just updating the idx");

            KnowledgeEntityEmbedding::redefine_hnsw_index(db, new_dimensions as usize).await?;
            return Ok(());
        }
        info!("Found {} entities to process.", total_entities);

        // Generate all new embeddings in memory
        let mut new_embeddings: HashMap<String, (Vec<f32>, String)> = HashMap::new();
        info!("Generating new embeddings for all entities...");
        for entity in all_entities.iter() {
            let embedding_input = format!(
                "name: {}, description: {}, type: {:?}",
                entity.name, entity.description, entity.entity_type
            );
            let retry_strategy = ExponentialBackoff::from_millis(100).map(jitter).take(3);

            let embedding = Retry::spawn(retry_strategy, || {
                crate::utils::embedding::generate_embedding_with_params(
                    openai_client,
                    &embedding_input,
                    new_model,
                    new_dimensions,
                )
            })
            .await?;

            // Check embedding lengths
            if embedding.len() != new_dimensions as usize {
                let err_msg = format!(
                "CRITICAL: Generated embedding for entity {} has incorrect dimension ({}). Expected {}. Aborting.",
                entity.id, embedding.len(), new_dimensions
            );
                error!("{}", err_msg);
                return Err(AppError::InternalError(err_msg));
            }
            new_embeddings.insert(entity.id.clone(), (embedding, entity.user_id.clone()));
        }
        info!("Successfully generated all new embeddings.");

        // Perform DB updates in a single transaction
        info!("Applying embedding updates in a transaction...");
        let mut transaction_query = String::from("BEGIN TRANSACTION;");

        // Add all update statements to the embedding table
        for (id, (embedding, user_id)) in new_embeddings {
            let embedding_str = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            transaction_query.push_str(&format!(
                "UPSERT type::thing('knowledge_entity_embedding', '{id}') SET \
                    entity_id = type::thing('knowledge_entity', '{id}'), \
                    embedding = {embedding}, \
                    user_id = '{user_id}', \
                    created_at = IF created_at != NONE THEN created_at ELSE time::now() END, \
                    updated_at = time::now();",
                id = id,
                embedding = embedding_str,
                user_id = user_id
            ));
        }

        transaction_query.push_str(&format!(
            "DEFINE INDEX OVERWRITE idx_embedding_knowledge_entity_embedding ON TABLE knowledge_entity_embedding FIELDS embedding HNSW DIMENSION {};",
            new_dimensions
        ));

        transaction_query.push_str("COMMIT TRANSACTION;");

        // Execute the entire atomic operation
        db.query(transaction_query).await?;

        info!("Re-embedding process for knowledge entities completed successfully.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::knowledge_entity_embedding::KnowledgeEntityEmbedding;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_knowledge_entity_creation() {
        // Create basic test entity
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
            entity_type.clone(),
            metadata.clone(),
            user_id.clone(),
        );

        // Verify all fields are set correctly
        assert_eq!(entity.source_id, source_id);
        assert_eq!(entity.name, name);
        assert_eq!(entity.description, description);
        assert_eq!(entity.entity_type, entity_type);
        assert_eq!(entity.metadata, metadata);
        assert_eq!(entity.user_id, user_id);
        assert!(!entity.id.is_empty());
    }

    #[tokio::test]
    async fn test_knowledge_entity_type_from_string() {
        // Test conversion from String to KnowledgeEntityType
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

        // Test default case
        assert_eq!(
            KnowledgeEntityType::from("unknown".to_string()),
            KnowledgeEntityType::Document
        );
    }

    #[tokio::test]
    async fn test_knowledge_entity_variants() {
        let variants = KnowledgeEntityType::variants();
        assert_eq!(variants.len(), 5);
        assert!(variants.contains(&"Idea"));
        assert!(variants.contains(&"Project"));
        assert!(variants.contains(&"Document"));
        assert!(variants.contains(&"Page"));
        assert!(variants.contains(&"TextSnippet"));
    }

    #[tokio::test]
    async fn test_delete_by_source_id() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        // Create two entities with the same source_id
        let source_id = "source123".to_string();
        let entity_type = KnowledgeEntityType::Document;
        let user_id = "user123".to_string();

        KnowledgeEntityEmbedding::redefine_hnsw_index(&db, 5)
            .await
            .expect("Failed to redefine index length");

        let entity1 = KnowledgeEntity::new(
            source_id.clone(),
            "Entity 1".to_string(),
            "Description 1".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        let entity2 = KnowledgeEntity::new(
            source_id.clone(),
            "Entity 2".to_string(),
            "Description 2".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        // Create an entity with a different source_id
        let different_source_id = "different_source".to_string();
        let different_entity = KnowledgeEntity::new(
            different_source_id.clone(),
            "Different Entity".to_string(),
            "Different Description".to_string(),
            entity_type.clone(),
            None,
            user_id.clone(),
        );

        let emb = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        // Store the entities
        KnowledgeEntity::store_with_embedding(entity1.clone(), emb.clone(), &db)
            .await
            .expect("Failed to store entity 1");
        KnowledgeEntity::store_with_embedding(entity2.clone(), emb.clone(), &db)
            .await
            .expect("Failed to store entity 2");
        KnowledgeEntity::store_with_embedding(different_entity.clone(), emb.clone(), &db)
            .await
            .expect("Failed to store different entity");

        // Delete by source_id
        KnowledgeEntity::delete_by_source_id(&source_id, &db)
            .await
            .expect("Failed to delete entities by source_id");

        // Verify all entities with the specified source_id are deleted
        let query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            KnowledgeEntity::table_name(),
            source_id
        );
        let remaining: Vec<KnowledgeEntity> = db
            .client
            .query(query)
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(
            remaining.len(),
            0,
            "All entities with the source_id should be deleted"
        );

        // Verify the entity with a different source_id still exists
        let different_query = format!(
            "SELECT * FROM {} WHERE source_id = '{}'",
            KnowledgeEntity::table_name(),
            different_source_id
        );
        let different_remaining: Vec<KnowledgeEntity> = db
            .client
            .query(different_query)
            .await
            .expect("Query failed")
            .take(0)
            .expect("Failed to get query results");
        assert_eq!(
            different_remaining.len(),
            1,
            "Entity with different source_id should still exist"
        );
        assert_eq!(different_remaining[0].id, different_entity.id);
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

        let results = KnowledgeEntity::vector_search(5, vec![0.1, 0.2, 0.3], &db, "user")
            .await
            .expect("vector search");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_vector_search_single_result() {
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
            .expect("store entity with embedding");

        let stored_entity: Option<KnowledgeEntity> = db.get_item(&entity.id).await.unwrap();
        assert!(stored_entity.is_some());

        let stored_embeddings: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {}",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .expect("query embeddings")
            .take(0)
            .expect("take embeddings");
        assert_eq!(stored_embeddings.len(), 1);

        let rid = surrealdb::RecordId::from_table_key(KnowledgeEntity::table_name(), &entity.id);
        let fetched_emb = KnowledgeEntityEmbedding::get_by_entity_id(&rid, &db)
            .await
            .expect("fetch embedding");
        assert!(fetched_emb.is_some());

        let results = KnowledgeEntity::vector_search(3, vec![0.1, 0.2, 0.3], &db, &user_id)
            .await
            .expect("vector search");

        assert_eq!(results.len(), 1);
        let res = &results[0];
        assert_eq!(res.entity.id, entity.id);
        assert_eq!(res.entity.source_id, source_id);
        assert_eq!(res.entity.name, "hello");
    }

    #[tokio::test]
    async fn test_vector_search_orders_by_similarity() {
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
            .expect("store e1");
        KnowledgeEntity::store_with_embedding(e2.clone(), vec![0.0, 1.0, 0.0], &db)
            .await
            .expect("store e2");

        let stored_e1: Option<KnowledgeEntity> = db.get_item(&e1.id).await.unwrap();
        let stored_e2: Option<KnowledgeEntity> = db.get_item(&e2.id).await.unwrap();
        assert!(stored_e1.is_some() && stored_e2.is_some());

        let stored_embeddings: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {}",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .expect("query embeddings")
            .take(0)
            .expect("take embeddings");
        assert_eq!(stored_embeddings.len(), 2);

        let rid_e1 = surrealdb::RecordId::from_table_key(KnowledgeEntity::table_name(), &e1.id);
        let rid_e2 = surrealdb::RecordId::from_table_key(KnowledgeEntity::table_name(), &e2.id);
        assert!(KnowledgeEntityEmbedding::get_by_entity_id(&rid_e1, &db)
            .await
            .unwrap()
            .is_some());
        assert!(KnowledgeEntityEmbedding::get_by_entity_id(&rid_e2, &db)
            .await
            .unwrap()
            .is_some());

        let results = KnowledgeEntity::vector_search(2, vec![0.0, 1.0, 0.0], &db, &user_id)
            .await
            .expect("vector search");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entity.id, e2.id);
        assert_eq!(results[1].entity.id, e1.id);
    }
}
