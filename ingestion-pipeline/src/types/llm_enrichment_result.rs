use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::task;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
            knowledge_relationship::KnowledgeRelationship,
        },
    },
    utils::embedding::generate_embedding,
};
use futures::future::try_join_all;

use crate::utils::GraphMapper;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMKnowledgeEntity {
    pub key: String, // Temporary identifier
    pub name: String,
    pub description: String,
    pub entity_type: String, // Should match KnowledgeEntityType variants
}

/// Represents a single relationship from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMRelationship {
    #[serde(rename = "type")]
    pub type_: String, // e.g., RelatedTo, RelevantTo
    pub source: String, // Key of the source entity
    pub target: String, // Key of the target entity
}

/// Represents the entire graph analysis result from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMEnrichmentResult {
    pub knowledge_entities: Vec<LLMKnowledgeEntity>,
    pub relationships: Vec<LLMRelationship>,
}

impl LLMEnrichmentResult {
    /// Converts the LLM graph analysis result into database entities and relationships.
    ///
    /// # Arguments
    ///
    /// * `source_id` - A UUID representing the source identifier.
    /// * `openai_client` - OpenAI client for LLM calls.
    ///
    /// # Returns
    ///
    /// * `Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError>` - A tuple containing vectors of `KnowledgeEntity` and `KnowledgeRelationship`.
    pub async fn to_database_entities(
        &self,
        source_id: &str,
        user_id: &str,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
        db_client: &SurrealDbClient,
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        // Create mapper and pre-assign IDs
        let mapper = Arc::new(Mutex::new(self.create_mapper()?));

        // Process entities
        let entities = self
            .process_entities(
                source_id,
                user_id,
                Arc::clone(&mapper),
                openai_client,
                db_client,
            )
            .await?;

        // Process relationships
        let relationships = self.process_relationships(source_id, user_id, Arc::clone(&mapper))?;

        Ok((entities, relationships))
    }

    fn create_mapper(&self) -> Result<GraphMapper, AppError> {
        let mut mapper = GraphMapper::new();

        // Pre-assign all IDs
        for entity in &self.knowledge_entities {
            mapper.assign_id(&entity.key);
        }

        Ok(mapper)
    }

    async fn process_entities(
        &self,
        source_id: &str,
        user_id: &str,
        mapper: Arc<Mutex<GraphMapper>>,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
        db_client: &SurrealDbClient,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        let futures: Vec<_> = self
            .knowledge_entities
            .iter()
            .map(|entity| {
                let mapper = Arc::clone(&mapper);
                let openai_client = openai_client.clone();
                let source_id = source_id.to_string();
                let user_id = user_id.to_string();
                let entity = entity.clone();
                let db_client = db_client.clone();

                task::spawn(async move {
                    create_single_entity(
                        &entity,
                        &source_id,
                        &user_id,
                        mapper,
                        &openai_client,
                        &db_client.clone(),
                    )
                    .await
                })
            })
            .collect();

        let results = try_join_all(futures)
            .await?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    fn process_relationships(
        &self,
        source_id: &str,
        user_id: &str,
        mapper: Arc<Mutex<GraphMapper>>,
    ) -> Result<Vec<KnowledgeRelationship>, AppError> {
        let mapper_guard = mapper
            .lock()
            .map_err(|_| AppError::GraphMapper("Failed to lock mapper".into()))?;
        self.relationships
            .iter()
            .map(|rel| {
                let source_db_id = mapper_guard.get_or_parse_id(&rel.source)?;
                let target_db_id = mapper_guard.get_or_parse_id(&rel.target)?;

                Ok(KnowledgeRelationship::new(
                    source_db_id.to_string(),
                    target_db_id.to_string(),
                    user_id.to_string(),
                    source_id.to_string(),
                    rel.type_.clone(),
                ))
            })
            .collect()
    }
}
async fn create_single_entity(
    llm_entity: &LLMKnowledgeEntity,
    source_id: &str,
    user_id: &str,
    mapper: Arc<Mutex<GraphMapper>>,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    db_client: &SurrealDbClient,
) -> Result<KnowledgeEntity, AppError> {
    let assigned_id = {
        let mapper = mapper
            .lock()
            .map_err(|_| AppError::GraphMapper("Failed to lock mapper".into()))?;
        mapper.get_id(&llm_entity.key)?.to_string()
    };

    let embedding_input = format!(
        "name: {}, description: {}, type: {}",
        llm_entity.name, llm_entity.description, llm_entity.entity_type
    );

    let embedding = generate_embedding(openai_client, &embedding_input, db_client).await?;

    let now = Utc::now();
    Ok(KnowledgeEntity {
        id: assigned_id,
        created_at: now,
        updated_at: now,
        name: llm_entity.name.to_string(),
        description: llm_entity.description.to_string(),
        entity_type: KnowledgeEntityType::from(llm_entity.entity_type.to_string()),
        source_id: source_id.to_string(),
        metadata: None,
        embedding,
        user_id: user_id.into(),
    })
}
