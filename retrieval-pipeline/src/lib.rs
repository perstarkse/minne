pub mod answer_retrieval;
pub mod answer_retrieval_helper;
pub mod fts;
pub mod graph;
pub mod pipeline;
pub mod reranking;
pub mod scoring;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk},
    },
};
use reranking::RerankerLease;
use tracing::instrument;

// Strategy output variants - defined before pipeline module
#[derive(Debug)]
pub enum StrategyOutput {
    Entities(Vec<RetrievedEntity>),
    Chunks(Vec<RetrievedChunk>),
}

pub use pipeline::{
    retrieved_entities_to_json, PipelineDiagnostics, PipelineStageTimings, RetrievalConfig,
    RetrievalStrategy, RetrievalTuning,
};

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

/// Primary orchestrator for the process of retrieving KnowledgeEntitities related to a input_text
#[instrument(skip_all, fields(user_id))]
pub async fn retrieve_entities(
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<StrategyOutput, AppError> {
    pipeline::run_pipeline(
        db_client,
        openai_client,
        None,
        input_text,
        user_id,
        config,
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
    use pipeline::{RetrievalConfig, RetrievalStrategy};
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
             REMOVE INDEX IF EXISTS idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding;
             DEFINE INDEX idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding FIELDS embedding HNSW DIMENSION 3;
             REMOVE INDEX IF EXISTS idx_embedding_knowledge_entity_embedding ON TABLE knowledge_entity_embedding;
             DEFINE INDEX idx_embedding_knowledge_entity_embedding ON TABLE knowledge_entity_embedding FIELDS embedding HNSW DIMENSION 3;
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
            user_id.into(),
        );
        let chunk = TextChunk::new(
            entity.source_id.clone(),
            "Tokio uses cooperative scheduling for fairness.".into(),
            user_id.into(),
        );

        KnowledgeEntity::store_with_embedding(entity.clone(), entity_embedding_high(), &db)
            .await
            .expect("Failed to store entity");
        TextChunk::store_with_embedding(chunk.clone(), chunk_embedding_primary(), &db)
            .await
            .expect("Failed to store chunk");

        let openai_client = Client::new();
        let results = pipeline::run_pipeline_with_embedding(
            &db,
            &openai_client,
            None,
            test_embedding(),
            "Rust concurrency async tasks",
            user_id,
            RetrievalConfig::default(),
            None,
        )
        .await
        .expect("Hybrid retrieval failed");

        let entities = match results {
            StrategyOutput::Entities(items) => items,
            other => panic!("expected entity results, got {:?}", other),
        };

        assert!(
            !entities.is_empty(),
            "Expected at least one retrieval result"
        );
        let top = &entities[0];
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
            user_id.into(),
        );
        let neighbor = KnowledgeEntity::new(
            "neighbor_source".into(),
            "Tokio Scheduler Deep Dive".into(),
            "Details on Tokio's cooperative scheduler.".into(),
            KnowledgeEntityType::Document,
            None,
            user_id.into(),
        );

        KnowledgeEntity::store_with_embedding(primary.clone(), entity_embedding_high(), &db)
            .await
            .expect("Failed to store primary entity");
        KnowledgeEntity::store_with_embedding(neighbor.clone(), entity_embedding_low(), &db)
            .await
            .expect("Failed to store neighbor entity");

        let primary_chunk = TextChunk::new(
            primary.source_id.clone(),
            "Rust async tasks use Tokio's cooperative scheduler.".into(),
            user_id.into(),
        );
        let neighbor_chunk = TextChunk::new(
            neighbor.source_id.clone(),
            "Tokio's scheduler manages task fairness across executors.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(primary_chunk, chunk_embedding_primary(), &db)
            .await
            .expect("Failed to store primary chunk");
        TextChunk::store_with_embedding(neighbor_chunk, chunk_embedding_secondary(), &db)
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
            None,
            test_embedding(),
            "Rust concurrency async tasks",
            user_id,
            RetrievalConfig::default(),
            None,
        )
        .await
        .expect("Hybrid retrieval failed");

        let entities = match results {
            StrategyOutput::Entities(items) => items,
            other => panic!("expected entity results, got {:?}", other),
        };

        let mut neighbor_entry = None;
        for entity in &entities {
            if entity.entity.id == neighbor.id {
                neighbor_entry = Some(entity.clone());
            }
        }

        println!("{:?}", entities);

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

    #[tokio::test]
    async fn test_revised_strategy_returns_chunks() {
        let db = setup_test_db().await;
        let user_id = "chunk_user";
        let chunk_one = TextChunk::new(
            "src_alpha".into(),
            "Tokio tasks execute on worker threads managed by the runtime.".into(),
            user_id.into(),
        );
        let chunk_two = TextChunk::new(
            "src_beta".into(),
            "Hyper utilizes Tokio to drive HTTP state machines efficiently.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(chunk_one.clone(), chunk_embedding_primary(), &db)
            .await
            .expect("Failed to store chunk one");
        TextChunk::store_with_embedding(chunk_two.clone(), chunk_embedding_secondary(), &db)
            .await
            .expect("Failed to store chunk two");

        let config = RetrievalConfig::with_strategy(RetrievalStrategy::Revised);
        let openai_client = Client::new();
        let results = pipeline::run_pipeline_with_embedding(
            &db,
            &openai_client,
            None,
            test_embedding(),
            "tokio runtime worker behavior",
            user_id,
            config,
            None,
        )
        .await
        .expect("Revised retrieval failed");

        let chunks = match results {
            StrategyOutput::Chunks(items) => items,
            other => panic!("expected chunk output, got {:?}", other),
        };

        assert!(
            !chunks.is_empty(),
            "Revised strategy should return chunk-only responses"
        );
        assert!(
            chunks
                .iter()
                .any(|entry| entry.chunk.chunk.contains("Tokio")),
            "Chunk results should contain relevant snippets"
        );
    }
}
