use std::sync::Arc;

use chrono::Utc;
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use anyhow::Context;
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
            knowledge_relationship::KnowledgeRelationship,
        },
    },
    utils::{embedding::generate_embedding, embedding::EmbeddingProvider},
};

use crate::pipeline::context::EmbeddedKnowledgeEntity;
use crate::utils::graph_mapper::GraphMapper;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMKnowledgeEntity {
    pub key: String,
    pub name: String,
    pub description: String,
    pub entity_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMRelationship {
    #[serde(rename = "type")]
    pub type_: String,
    pub source: String,
    pub target: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMEnrichmentResult {
    pub knowledge_entities: Vec<LLMKnowledgeEntity>,
    pub relationships: Vec<LLMRelationship>,
}

impl LLMEnrichmentResult {
    pub async fn to_database_entities(
        &self,
        source_id: &str,
        user_id: &str,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
        db_client: &SurrealDbClient,
        entity_concurrency: usize,
        embedding_provider: Option<&EmbeddingProvider>,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        let mapper = Arc::new(self.create_mapper());

        let entities = self
            .process_entities(
                source_id,
                user_id,
                Arc::clone(&mapper),
                openai_client,
                db_client,
                entity_concurrency,
                embedding_provider,
            )
            .await?;

        let relationships = self.process_relationships(source_id, user_id, mapper.as_ref())?;

        Ok((entities, relationships))
    }

    fn create_mapper(&self) -> GraphMapper {
        let mut mapper = GraphMapper::new();

        for entity in &self.knowledge_entities {
            mapper.assign_id(&entity.key);
        }

        mapper
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_entities(
        &self,
        source_id: &str,
        user_id: &str,
        mapper: Arc<GraphMapper>,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
        db_client: &SurrealDbClient,
        entity_concurrency: usize,
        embedding_provider: Option<&EmbeddingProvider>,
    ) -> Result<Vec<EmbeddedKnowledgeEntity>, AppError> {
        stream::iter(self.knowledge_entities.clone().into_iter().map(|entity| {
            let mapper = Arc::clone(&mapper);
            let openai_client = openai_client.clone();
            let source_id = source_id.to_string();
            let user_id = user_id.to_string();
            let db_client = db_client.clone();

            async move {
                create_single_entity(
                    &entity,
                    &source_id,
                    &user_id,
                    mapper,
                    &openai_client,
                    &db_client,
                    embedding_provider,
                )
                .await
            }
        }))
        .buffer_unordered(entity_concurrency.max(1))
        .try_collect()
        .await
    }

    fn process_relationships(
        &self,
        source_id: &str,
        user_id: &str,
        mapper: &GraphMapper,
    ) -> Result<Vec<KnowledgeRelationship>, AppError> {
        self.relationships
            .iter()
            .map(|rel| {
                let source_db_id = mapper.get_or_parse_id(&rel.source)?;
                let target_db_id = mapper.get_or_parse_id(&rel.target)?;

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
    mapper: Arc<GraphMapper>,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    db_client: &SurrealDbClient,
    embedding_provider: Option<&EmbeddingProvider>,
) -> Result<EmbeddedKnowledgeEntity, AppError> {
    let assigned_id = mapper.get_id(&llm_entity.key)?.to_string();

    let embedding_input = format!(
        "name: {}, description: {}, type: {}",
        llm_entity.name, llm_entity.description, llm_entity.entity_type
    );

    let embedding = if let Some(provider) = embedding_provider {
        provider
            .embed(&embedding_input)
            .await
            .context("generating FastEmbed embedding for entity")?
    } else {
        generate_embedding(openai_client, &embedding_input, db_client).await?
    };

    let now = Utc::now();
    let entity = KnowledgeEntity {
        id: assigned_id,
        created_at: now,
        updated_at: now,
        name: llm_entity.name.clone(),
        description: llm_entity.description.clone(),
        entity_type: KnowledgeEntityType::from(llm_entity.entity_type.clone()),
        source_id: source_id.to_string(),
        metadata: None,
        user_id: user_id.into(),
    };

    Ok(EmbeddedKnowledgeEntity { entity, embedding })
}
