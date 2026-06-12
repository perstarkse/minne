use chrono::Utc;
use serde::{Deserialize, Serialize};

use common::{
    error::AppError,
    storage::types::{
        knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
        knowledge_relationship::KnowledgeRelationship,
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
        embedding_provider: &EmbeddingProvider,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        let mapper = self.create_mapper();

        let entities = self
            .process_entities(source_id, user_id, &mapper, embedding_provider)
            .await?;

        let relationships = self.process_relationships(source_id, user_id, &mapper)?;

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
        mapper: &GraphMapper,
        embedding_provider: &EmbeddingProvider,
    ) -> Result<Vec<EmbeddedKnowledgeEntity>, AppError> {
        if self.knowledge_entities.is_empty() {
            return Ok(Vec::new());
        }

        let now = Utc::now();
        let mut prepared = Vec::with_capacity(self.knowledge_entities.len());
        let mut embedding_inputs = Vec::with_capacity(self.knowledge_entities.len());

        for llm_entity in &self.knowledge_entities {
            let assigned_id = mapper.get_id(&llm_entity.key)?.to_string();
            let entity_type = KnowledgeEntityType::from(llm_entity.entity_type.clone());
            embedding_inputs.push(KnowledgeEntity::embedding_input_text(
                &llm_entity.name,
                &llm_entity.description,
                entity_type,
            ));
            prepared.push((llm_entity, assigned_id, entity_type));
        }

        // Embed all entities from this document in one batch: a single lock acquisition and one
        // blocking hop, letting the backend batch the inference internally.
        let embeddings = embedding_provider
            .embed_batch(&embedding_inputs)
            .await
            .map_err(|e| AppError::InternalError(format!("entity embedding batch failed: {e}")))?;

        if embeddings.len() != prepared.len() {
            return Err(AppError::InternalError(format!(
                "embedding batch returned {} vectors for {} entities",
                embeddings.len(),
                prepared.len()
            )));
        }

        let mut entities = Vec::with_capacity(prepared.len());
        for ((llm_entity, assigned_id, entity_type), embedding) in
            prepared.into_iter().zip(embeddings)
        {
            entities.push(EmbeddedKnowledgeEntity {
                entity: KnowledgeEntity {
                    id: assigned_id,
                    created_at: now,
                    updated_at: now,
                    name: llm_entity.name.clone(),
                    description: llm_entity.description.clone(),
                    entity_type,
                    source_id: source_id.to_string(),
                    metadata: None,
                    user_id: user_id.to_string(),
                },
                embedding,
            });
        }

        Ok(entities)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use common::utils::embedding::EmbeddingProvider;
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

    #[tokio::test]
    async fn process_entities_batches_embeddings_and_preserves_order() -> anyhow::Result<()> {
        let result = LLMEnrichmentResult {
            knowledge_entities: vec![entity("k1"), entity("k2"), entity("k3")],
            relationships: Vec::new(),
        };
        let mapper = result.create_mapper();
        let provider = EmbeddingProvider::new_hashed(8)?;

        let entities = result
            .process_entities("source-1", "user-1", &mapper, &provider)
            .await?;

        assert_eq!(entities.len(), 3);
        let first = entities.first().expect("first entity");
        let second = entities.get(1).expect("second entity");
        let third = entities.get(2).expect("third entity");
        assert_eq!(first.entity.name, "name-k1");
        assert_eq!(second.entity.name, "name-k2");
        assert_eq!(third.entity.name, "name-k3");
        assert!(entities.iter().all(|item| item.embedding.len() == 8));
        assert_ne!(first.embedding, second.embedding);

        Ok(())
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
