//! Shared helpers for ingestion pipeline integration and persistence tests.

use chrono::Utc;
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
            knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk,
            text_content::TextContent,
        },
    },
};
use uuid::Uuid;

use super::{
    config::IngestionTuning,
    context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk, PipelineArtifacts},
    persistence::persist_artifacts,
};

pub const TEST_EMBEDDING_DIM: usize = 3;

pub async fn setup_db() -> anyhow::Result<SurrealDbClient> {
    let namespace = "ingestion_pipeline_test";
    let database = Uuid::new_v4().to_string();
    let db = SurrealDbClient::memory(namespace, &database).await?;
    db.apply_migrations().await?;
    Ok(db)
}

pub fn tuning() -> IngestionTuning {
    IngestionTuning::default()
}

pub fn sample_artifacts(source_id: &str, user_id: &str) -> PipelineArtifacts {
    large_artifacts(source_id, user_id, 1, 1, 1, TEST_EMBEDDING_DIM)
}

#[allow(
    clippy::too_many_arguments,
    clippy::arithmetic_side_effects,
    clippy::expect_used
)]
pub fn large_artifacts(
    source_id: &str,
    user_id: &str,
    chunk_count: usize,
    entity_count: usize,
    relationship_count: usize,
    embedding_dim: usize,
) -> PipelineArtifacts {
    let now = Utc::now();
    let embedding = vec![0.1_f32; embedding_dim];

    let mut entities = Vec::with_capacity(entity_count);
    let mut entity_ids = Vec::with_capacity(entity_count);

    for index in 0..entity_count {
        let entity_id = Uuid::new_v4().to_string();
        entity_ids.push(entity_id.clone());
        entities.push(EmbeddedKnowledgeEntity {
            entity: KnowledgeEntity {
                id: entity_id,
                created_at: now,
                updated_at: now,
                name: format!("entity-{index}"),
                description: format!("description-{index}"),
                entity_type: KnowledgeEntityType::Idea,
                source_id: source_id.to_string(),
                metadata: None,
                user_id: user_id.to_string(),
            },
            embedding: embedding.clone(),
        });
    }

    let mut relationships = Vec::with_capacity(relationship_count);
    assert!(
        entity_count > 0 || relationship_count == 0,
        "large_artifacts requires entity_count > 0 when relationship_count > 0"
    );
    for index in 0..relationship_count {
        let in_id = entity_ids
            .get(index % entity_count)
            .expect("entity_count > 0 when relationship_count > 0")
            .clone();
        let out_id = entity_ids
            .get((index + 1) % entity_count)
            .expect("entity_count > 0 when relationship_count > 0")
            .clone();
        relationships.push(KnowledgeRelationship::new(
            in_id,
            out_id,
            user_id.to_string(),
            source_id.to_string(),
            "relates_to".to_string(),
        ));
    }

    let mut chunks = Vec::with_capacity(chunk_count);
    for index in 0..chunk_count {
        chunks.push(EmbeddedTextChunk {
            chunk: TextChunk {
                id: Uuid::new_v4().to_string(),
                created_at: now,
                updated_at: now,
                source_id: source_id.to_string(),
                chunk: format!("chunk body {index}"),
                user_id: user_id.to_string(),
            },
            embedding: embedding.clone(),
        });
    }

    PipelineArtifacts {
        text_content: TextContent {
            id: source_id.to_string(),
            created_at: now,
            updated_at: now,
            text: format!("document with {chunk_count} chunks"),
            file_info: None,
            url_info: None,
            context: None,
            category: "notes".to_string(),
            user_id: user_id.to_string(),
        },
        entities,
        relationships,
        chunks,
    }
}

pub async fn persist(db: &SurrealDbClient, artifacts: PipelineArtifacts) -> Result<(), AppError> {
    persist_artifacts(db, &tuning(), TEST_EMBEDDING_DIM, artifacts).await?;
    Ok(())
}

pub async fn count_chunks_for_source(
    db: &SurrealDbClient,
    source_id: &str,
) -> anyhow::Result<usize> {
    let chunks: Vec<TextChunk> = db
        .client
        .query("SELECT * FROM text_chunk WHERE source_id = $source_id;")
        .bind(("source_id", source_id.to_string()))
        .await?
        .take(0)?;
    Ok(chunks.len())
}

pub async fn count_entities_for_source(
    db: &SurrealDbClient,
    source_id: &str,
) -> anyhow::Result<usize> {
    let entities: Vec<KnowledgeEntity> = db
        .client
        .query("SELECT * FROM knowledge_entity WHERE source_id = $source_id;")
        .bind(("source_id", source_id.to_string()))
        .await?
        .take(0)?;
    Ok(entities.len())
}

pub async fn count_relationships_for_source(
    db: &SurrealDbClient,
    source_id: &str,
) -> anyhow::Result<usize> {
    let relationships: Vec<KnowledgeRelationship> = db
        .client
        .query("SELECT * FROM relates_to WHERE metadata.source_id = $source_id;")
        .bind(("source_id", source_id.to_string()))
        .await?
        .take(0)?;
    Ok(relationships.len())
}
