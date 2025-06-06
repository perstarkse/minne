use std::collections::HashMap;

use crate::{
    error::AppError, storage::db::SurrealDbClient, stored_object,
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

stored_object!(KnowledgeEntity, "knowledge_entity", {
    source_id: String,
    name: String,
    description: String,
    entity_type: KnowledgeEntityType,
    metadata: Option<serde_json::Value>,
    embedding: Vec<f32>,
    user_id: String
});

impl KnowledgeEntity {
    pub fn new(
        source_id: String,
        name: String,
        description: String,
        entity_type: KnowledgeEntityType,
        metadata: Option<serde_json::Value>,
        embedding: Vec<f32>,
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
            embedding,
            user_id,
        }
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

        db_client
            .client
            .query(
                "UPDATE type::thing($table, $id)
                SET name = $name,
                    description = $description,
                    updated_at = $updated_at,
                    entity_type = $entity_type,
                    embedding = $embedding
                RETURN AFTER",
            )
            .bind(("table", Self::table_name()))
            .bind(("id", id.to_string()))
            .bind(("name", name.to_string()))
            .bind(("updated_at", Utc::now()))
            .bind(("entity_type", entity_type.to_owned()))
            .bind(("embedding", embedding))
            .bind(("description", description.to_string()))
            .await?;

        Ok(())
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
            info!("No knowledge entities to update. Skipping.");
            return Ok(());
        }
        info!("Found {} entities to process.", total_entities);

        // Generate all new embeddings in memory
        let mut new_embeddings: HashMap<String, Vec<f32>> = HashMap::new();
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
            new_embeddings.insert(entity.id.clone(), embedding);
        }
        info!("Successfully generated all new embeddings.");

        // Perform DB updates in a single transaction
        info!("Applying schema and data changes in a transaction...");
        let mut transaction_query = String::from("BEGIN TRANSACTION;");

        // Add all update statements
        for (id, embedding) in new_embeddings {
            // We must properly serialize the vector for the SurrealQL query string
            let embedding_str = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            transaction_query.push_str(&format!(
            "UPDATE type::thing('knowledge_entity', '{}') SET embedding = {}, updated_at = time::now();",
            id, embedding_str
        ));
        }

        // Re-create the index after updating the data that it will index
        transaction_query
            .push_str("REMOVE INDEX idx_embedding_entities ON TABLE knowledge_entity;");
        transaction_query.push_str(&format!(
        "DEFINE INDEX idx_embedding_entities ON TABLE knowledge_entity FIELDS embedding HNSW DIMENSION {};",
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
    use serde_json::json;

    #[tokio::test]
    async fn test_knowledge_entity_creation() {
        // Create basic test entity
        let source_id = "source123".to_string();
        let name = "Test Entity".to_string();
        let description = "Test Description".to_string();
        let entity_type = KnowledgeEntityType::Document;
        let metadata = Some(json!({"key": "value"}));
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let user_id = "user123".to_string();

        let entity = KnowledgeEntity::new(
            source_id.clone(),
            name.clone(),
            description.clone(),
            entity_type.clone(),
            metadata.clone(),
            embedding.clone(),
            user_id.clone(),
        );

        // Verify all fields are set correctly
        assert_eq!(entity.source_id, source_id);
        assert_eq!(entity.name, name);
        assert_eq!(entity.description, description);
        assert_eq!(entity.entity_type, entity_type);
        assert_eq!(entity.metadata, metadata);
        assert_eq!(entity.embedding, embedding);
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

        // Create two entities with the same source_id
        let source_id = "source123".to_string();
        let entity_type = KnowledgeEntityType::Document;
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let user_id = "user123".to_string();

        let entity1 = KnowledgeEntity::new(
            source_id.clone(),
            "Entity 1".to_string(),
            "Description 1".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
            user_id.clone(),
        );

        let entity2 = KnowledgeEntity::new(
            source_id.clone(),
            "Entity 2".to_string(),
            "Description 2".to_string(),
            entity_type.clone(),
            None,
            embedding.clone(),
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
            embedding.clone(),
            user_id.clone(),
        );

        // Store the entities
        db.store_item(entity1)
            .await
            .expect("Failed to store entity 1");
        db.store_item(entity2)
            .await
            .expect("Failed to store entity 2");
        db.store_item(different_entity.clone())
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

    // Note: We can't easily test the patch method without mocking the OpenAI client
    // and the generate_embedding function. This would require more complex setup.
}
