use std::sync::Arc;

use chrono::Utc;
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use common::{
    error::AppError,
    storage::{
        types::{
            knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
            knowledge_relationship::KnowledgeRelationship,
        },
    },
    utils::embedding::EmbeddingProvider,
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
        entity_concurrency: usize,
        embedding_provider: &EmbeddingProvider,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        let mapper = Arc::new(self.create_mapper());

        let entities = self
            .process_entities(
                source_id,
                user_id,
                Arc::clone(&mapper),
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

    async fn process_entities(
        &self,
        source_id: &str,
        user_id: &str,
        mapper: Arc<GraphMapper>,
        entity_concurrency: usize,
        embedding_provider: &EmbeddingProvider,
    ) -> Result<Vec<EmbeddedKnowledgeEntity>, AppError> {
        stream::iter(self.knowledge_entities.clone().into_iter().map(|entity| {
            let mapper = Arc::clone(&mapper);
            let source_id = source_id.to_string();
            let user_id = user_id.to_string();

            async move {
                create_single_entity(
                    &entity,
                    &source_id,
                    &user_id,
                    mapper,
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
    embedding_provider: &EmbeddingProvider,
) -> Result<EmbeddedKnowledgeEntity, AppError> {
    let assigned_id = mapper.get_id(&llm_entity.key)?.to_string();

    let embedding_input = format!(
        "name: {}, description: {}, type: {}",
        llm_entity.name, llm_entity.description, llm_entity.entity_type
    );

    let embedding = embedding_provider.embed(&embedding_input).await?;

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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use uuid::Uuid;

    fn entity(key: &str) -> LLMKnowledgeEntity {
        LLMKnowledgeEntity {
            key: key.to_string(),
            name: format!("name-{key}"),
            description: format!("desc-{key}"),
            entity_type: "Idea".to_string(),
        }
    }

    fn relationship(type_: &str, source: &str, target: &str) -> LLMRelationship {
        LLMRelationship {
            type_: type_.to_string(),
            source: source.to_string(),
            target: target.to_string(),
        }
    }

    #[test]
    fn create_mapper_assigns_id_per_entity_key() {
        let result = LLMEnrichmentResult {
            knowledge_entities: vec![entity("k1"), entity("k2")],
            relationships: Vec::new(),
        };

        let mapper = result.create_mapper();

        assert!(mapper.get_id("k1").is_ok());
        assert!(mapper.get_id("k2").is_ok());
        assert_ne!(
            mapper.get_id("k1").expect("k1"),
            mapper.get_id("k2").expect("k2")
        );
    }

    #[test]
    fn process_relationships_resolves_keys_to_assigned_ids() {
        let result = LLMEnrichmentResult {
            knowledge_entities: vec![entity("k1"), entity("k2")],
            relationships: vec![relationship("relates_to", "k1", "k2")],
        };
        let mapper = result.create_mapper();

        let relationships = result
            .process_relationships("source-1", "user-1", &mapper)
            .expect("relationships resolve");

        assert_eq!(relationships.len(), 1);
        let rel = relationships.first().expect("one relationship");
        assert_eq!(rel.in_, mapper.get_id("k1").expect("k1").to_string());
        assert_eq!(rel.out, mapper.get_id("k2").expect("k2").to_string());
        assert_eq!(rel.metadata.relationship_type, "relates_to");
        assert_eq!(rel.metadata.source_id, "source-1");
        assert_eq!(rel.metadata.user_id, "user-1");
    }

    #[test]
    fn process_relationships_accepts_raw_uuid_endpoints() {
        let raw = Uuid::new_v4();
        let result = LLMEnrichmentResult {
            knowledge_entities: vec![entity("k1")],
            relationships: vec![relationship("relates_to", "k1", &raw.to_string())],
        };
        let mapper = result.create_mapper();

        let relationships = result
            .process_relationships("source-1", "user-1", &mapper)
            .expect("raw uuid target resolves");

        assert_eq!(
            relationships.first().expect("one relationship").out,
            raw.to_string()
        );
    }

    #[test]
    fn process_relationships_errors_on_unknown_endpoint() {
        let result = LLMEnrichmentResult {
            knowledge_entities: vec![entity("k1")],
            relationships: vec![relationship("relates_to", "k1", "missing-key")],
        };
        let mapper = result.create_mapper();

        assert!(matches!(
            result.process_relationships("source-1", "user-1", &mapper),
            Err(AppError::GraphMapper(_))
        ));
    }
}
