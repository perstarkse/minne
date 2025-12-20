pub mod answer_retrieval;
pub mod answer_retrieval_helper;

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
    Search(SearchResult),
}

/// Unified search result containing both chunks and entities
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunks: Vec<RetrievedChunk>,
    pub entities: Vec<RetrievedEntity>,
}

impl SearchResult {
    pub fn new(chunks: Vec<RetrievedChunk>, entities: Vec<RetrievedEntity>) -> Self {
        Self { chunks, entities }
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.entities.is_empty()
    }
}

pub use pipeline::{
    retrieved_entities_to_json, PipelineDiagnostics, PipelineStageTimings, RetrievalConfig,
    RetrievalStrategy, RetrievalTuning, SearchTarget,
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
    embedding_provider: Option<&common::utils::embedding::EmbeddingProvider>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<StrategyOutput, AppError> {
    pipeline::run_pipeline(
        db_client,
        openai_client,
        embedding_provider,
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
    use common::storage::indexes::ensure_runtime_indexes;
    use common::storage::types::text_chunk::TextChunk;
    use pipeline::{RetrievalConfig, RetrievalStrategy};
    use uuid::Uuid;

    fn test_embedding() -> Vec<f32> {
        vec![0.9, 0.1, 0.0]
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

        ensure_runtime_indexes(&db, 3)
            .await
            .expect("failed to build runtime indexes");

        db
    }

    #[tokio::test]
    async fn test_default_strategy_retrieves_chunks() {
        let db = setup_test_db().await;
        let user_id = "test_user";
        let chunk = TextChunk::new(
            "source_1".into(),
            "Tokio uses cooperative scheduling for fairness.".into(),
            user_id.into(),
        );

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
        .expect("Default strategy retrieval failed");

        let chunks = match results {
            StrategyOutput::Chunks(items) => items,
            other => panic!("expected chunk results, got {:?}", other),
        };

        assert!(!chunks.is_empty(), "Expected at least one retrieval result");
        assert!(
            chunks[0].chunk.chunk.contains("Tokio"),
            "Expected chunk about Tokio"
        );
    }

    #[tokio::test]
    async fn test_default_strategy_returns_chunks_from_multiple_sources() {
        let db = setup_test_db().await;
        let user_id = "multi_source_user";

        let primary_chunk = TextChunk::new(
            "primary_source".into(),
            "Rust async tasks use Tokio's cooperative scheduler.".into(),
            user_id.into(),
        );
        let secondary_chunk = TextChunk::new(
            "secondary_source".into(),
            "Tokio's scheduler manages task fairness across executors.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(primary_chunk, chunk_embedding_primary(), &db)
            .await
            .expect("Failed to store primary chunk");
        TextChunk::store_with_embedding(secondary_chunk, chunk_embedding_secondary(), &db)
            .await
            .expect("Failed to store secondary chunk");

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
        .expect("Default strategy retrieval failed");

        let chunks = match results {
            StrategyOutput::Chunks(items) => items,
            other => panic!("expected chunk results, got {:?}", other),
        };

        assert!(chunks.len() >= 2, "Expected chunks from multiple sources");
        assert!(
            chunks.iter().any(|c| c.chunk.source_id == "primary_source"),
            "Should include primary source chunk"
        );
        assert!(
            chunks
                .iter()
                .any(|c| c.chunk.source_id == "secondary_source"),
            "Should include secondary source chunk"
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

        let config = RetrievalConfig::with_strategy(RetrievalStrategy::Default);
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

    #[tokio::test]
    async fn test_search_strategy_returns_search_result() {
        let db = setup_test_db().await;
        let user_id = "search_user";
        let chunk = TextChunk::new(
            "search_src".into(),
            "Async Rust programming uses Tokio runtime for concurrent tasks.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(chunk.clone(), chunk_embedding_primary(), &db)
            .await
            .expect("Failed to store chunk");

        let config = RetrievalConfig::for_search(pipeline::SearchTarget::Both);
        let openai_client = Client::new();
        let results = pipeline::run_pipeline_with_embedding(
            &db,
            &openai_client,
            None,
            test_embedding(),
            "async rust programming",
            user_id,
            config,
            None,
        )
        .await
        .expect("Search strategy retrieval failed");

        let search_result = match results {
            StrategyOutput::Search(sr) => sr,
            other => panic!("expected Search output, got {:?}", other),
        };

        // Should return chunks (entities may be empty if none stored)
        assert!(
            !search_result.chunks.is_empty(),
            "Search strategy should return chunks"
        );
        assert!(
            search_result.chunks.iter().any(|c| c.chunk.chunk.contains("Tokio")),
            "Search results should contain relevant chunks"
        );
    }
}
