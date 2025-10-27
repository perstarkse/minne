pub mod answer_retrieval;
pub mod answer_retrieval_helper;
pub mod fts;
pub mod graph;
pub mod pipeline;
pub mod reranking;
pub mod scoring;
pub mod vector;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk},
    },
};
use reranking::RerankerLease;
use tracing::instrument;

pub use pipeline::{retrieved_entities_to_json, RetrievalConfig, RetrievalTuning};

// Captures a supporting chunk plus its fused retrieval score for downstream prompts.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub chunk: TextChunk,
    pub score: f32,
}

// Final entity representation returned to callers, enriched with ranked chunks.
#[derive(Debug, Clone)]
pub struct RetrievedEntity {
    pub entity: KnowledgeEntity,
    pub score: f32,
    pub chunks: Vec<RetrievedChunk>,
}

// Primary orchestrator for the process of retrieving KnowledgeEntitities related to a input_text
#[instrument(skip_all, fields(user_id))]
pub async fn retrieve_entities(
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    input_text: &str,
    user_id: &str,
    reranker: Option<RerankerLease>,
) -> Result<Vec<RetrievedEntity>, AppError> {
    pipeline::run_pipeline(
        db_client,
        openai_client,
        input_text,
        user_id,
        RetrievalConfig::default(),
        reranker,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_openai::Client;
    use common::storage::types::{
        knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
        knowledge_relationship::KnowledgeRelationship,
        text_chunk::TextChunk,
    };
    use pipeline::RetrievalConfig;
    use uuid::Uuid;

    fn test_embedding() -> Vec<f32> {
        vec![0.9, 0.1, 0.0]
    }

    fn entity_embedding_high() -> Vec<f32> {
        vec![0.8, 0.2, 0.0]
    }

    fn entity_embedding_low() -> Vec<f32> {
        vec![0.1, 0.9, 0.0]
    }

    fn chunk_embedding_primary() -> Vec<f32> {
        vec![0.85, 0.15, 0.0]
    }

    fn chunk_embedding_secondary() -> Vec<f32> {
        vec![0.2, 0.8, 0.0]
    }

    async fn setup_test_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        db.query(
            "BEGIN TRANSACTION;
             REMOVE INDEX IF EXISTS idx_embedding_chunks ON TABLE text_chunk;
             DEFINE INDEX idx_embedding_chunks ON TABLE text_chunk FIELDS embedding HNSW DIMENSION 3;
             REMOVE INDEX IF EXISTS idx_embedding_entities ON TABLE knowledge_entity;
             DEFINE INDEX idx_embedding_entities ON TABLE knowledge_entity FIELDS embedding HNSW DIMENSION 3;
             COMMIT TRANSACTION;",
        )
        .await
        .expect("Failed to configure indices");

        db
    }

    #[tokio::test]
    async fn test_retrieve_entities_with_embedding_basic_flow() {
        let db = setup_test_db().await;
        let user_id = "test_user";
        let entity = KnowledgeEntity::new(
            "source_1".into(),
            "Rust async guide".into(),
            "Detailed notes about async runtimes".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_high(),
            user_id.into(),
        );
        let chunk = TextChunk::new(
            entity.source_id.clone(),
            "Tokio uses cooperative scheduling for fairness.".into(),
            chunk_embedding_primary(),
            user_id.into(),
        );

        db.store_item(entity.clone())
            .await
            .expect("Failed to store entity");
        db.store_item(chunk.clone())
            .await
            .expect("Failed to store chunk");

        let openai_client = Client::new();
        let results = pipeline::run_pipeline_with_embedding(
            &db,
            &openai_client,
            test_embedding(),
            "Rust concurrency async tasks",
            user_id,
            RetrievalConfig::default(),
            None,
        )
        .await
        .expect("Hybrid retrieval failed");

        assert!(
            !results.is_empty(),
            "Expected at least one retrieval result"
        );
        let top = &results[0];
        assert!(
            top.entity.name.contains("Rust"),
            "Expected Rust entity to be ranked first"
        );
        assert!(
            !top.chunks.is_empty(),
            "Expected Rust entity to include supporting chunks"
        );
    }

    #[tokio::test]
    async fn test_graph_relationship_enriches_results() {
        let db = setup_test_db().await;
        let user_id = "graph_user";

        let primary = KnowledgeEntity::new(
            "primary_source".into(),
            "Async Rust patterns".into(),
            "Explores async runtimes and scheduling strategies.".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_high(),
            user_id.into(),
        );
        let neighbor = KnowledgeEntity::new(
            "neighbor_source".into(),
            "Tokio Scheduler Deep Dive".into(),
            "Details on Tokio's cooperative scheduler.".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_low(),
            user_id.into(),
        );

        db.store_item(primary.clone())
            .await
            .expect("Failed to store primary entity");
        db.store_item(neighbor.clone())
            .await
            .expect("Failed to store neighbor entity");

        let primary_chunk = TextChunk::new(
            primary.source_id.clone(),
            "Rust async tasks use Tokio's cooperative scheduler.".into(),
            chunk_embedding_primary(),
            user_id.into(),
        );
        let neighbor_chunk = TextChunk::new(
            neighbor.source_id.clone(),
            "Tokio's scheduler manages task fairness across executors.".into(),
            chunk_embedding_secondary(),
            user_id.into(),
        );

        db.store_item(primary_chunk)
            .await
            .expect("Failed to store primary chunk");
        db.store_item(neighbor_chunk)
            .await
            .expect("Failed to store neighbor chunk");

        let openai_client = Client::new();
        let relationship = KnowledgeRelationship::new(
            primary.id.clone(),
            neighbor.id.clone(),
            user_id.into(),
            "relationship_source".into(),
            "references".into(),
        );
        relationship
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship");

        let results = pipeline::run_pipeline_with_embedding(
            &db,
            &openai_client,
            test_embedding(),
            "Rust concurrency async tasks",
            user_id,
            RetrievalConfig::default(),
            None,
        )
        .await
        .expect("Hybrid retrieval failed");

        let mut neighbor_entry = None;
        for entity in &results {
            if entity.entity.id == neighbor.id {
                neighbor_entry = Some(entity.clone());
            }
        }

        let neighbor_entry =
            neighbor_entry.expect("Graph-enriched neighbor should appear in results");

        assert!(
            neighbor_entry.score > 0.2,
            "Graph-enriched entity should have a meaningful fused score"
        );
        assert!(
            neighbor_entry
                .chunks
                .iter()
                .all(|chunk| chunk.chunk.source_id == neighbor.source_id),
            "Neighbor entity should surface its own supporting chunks"
        );
    }
}
