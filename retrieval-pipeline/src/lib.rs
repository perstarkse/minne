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
    retrieved_entities_to_json, Diagnostics, StageTimings, RetrievalConfig,
    RetrievalStrategy, RetrievalTuning, RetrievalTuningFlags, SearchTarget,
};

// Backward-compatible type aliases for external consumers
pub type PipelineDiagnostics = Diagnostics;
pub type PipelineStageTimings = StageTimings;

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

/// Primary orchestrator for the process of retrieving `KnowledgeEntity` values related to an `input_text`
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
    let params = pipeline::StrategyParams {
        db_client,
        openai_client,
        embedding_provider,
        input_text,
        user_id,
        config,
        reranker,
    };
    pipeline::execute(params).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{self};
    use async_openai::Client;
    use common::storage::indexes::ensure_runtime;
    use common::storage::types::system_settings::SystemSettings;
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

    async fn configure_embedding_dimension(
        db: &SurrealDbClient,
        dimension: u32,
    ) -> anyhow::Result<()> {
        let mut settings = SystemSettings::get_current(db).await?;
        settings.embedding_dimensions = dimension;
        SystemSettings::update(db, settings).await?;
        Ok(())
    }

    async fn setup_test_db() -> anyhow::Result<SurrealDbClient> {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database).await?;

        db.apply_migrations().await?;

        configure_embedding_dimension(&db, 3).await?;
        ensure_runtime(&db, 3).await?;

        Ok(db)
    }

    #[tokio::test]
    async fn test_default_strategy_retrieves_chunks() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "test_user";
        let chunk = TextChunk::new(
            "source_1".into(),
            "Tokio uses cooperative scheduling for fairness.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(chunk.clone(), chunk_embedding_primary(), &db).await?;

        let openai_client = Client::new();
        let params = pipeline::StrategyParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "Rust concurrency async tasks",
            user_id,
            config: RetrievalConfig::default(),
            reranker: None,
        };
        let results = pipeline::run_pipeline_with_embedding(params, test_embedding())
            .await?;

        let chunks = match results {
            StrategyOutput::Chunks(items) => items,
            other => anyhow::bail!("expected chunk results, got {other:?}"),
        };

        assert!(!chunks.is_empty(), "Expected at least one retrieval result");
        assert!(
            chunks.first().is_some_and(|c| c.chunk.chunk.contains("Tokio")),
            "Expected chunk about Tokio"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_default_strategy_returns_chunks_from_multiple_sources(
    ) -> anyhow::Result<()> {
        let db = setup_test_db().await?;
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

        TextChunk::store_with_embedding(primary_chunk, chunk_embedding_primary(), &db).await?;
        TextChunk::store_with_embedding(secondary_chunk, chunk_embedding_secondary(), &db).await?;

        let openai_client = Client::new();
        let params = pipeline::StrategyParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "Rust concurrency async tasks",
            user_id,
            config: RetrievalConfig::default(),
            reranker: None,
        };
        let results = pipeline::run_pipeline_with_embedding(params, test_embedding())
            .await?;

        let chunks = match results {
            StrategyOutput::Chunks(items) => items,
            other => anyhow::bail!("expected chunk results, got {other:?}"),
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
        Ok(())
    }

    #[tokio::test]
    async fn test_revised_strategy_returns_chunks() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
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

        TextChunk::store_with_embedding(chunk_one.clone(), chunk_embedding_primary(), &db).await?;
        TextChunk::store_with_embedding(chunk_two.clone(), chunk_embedding_secondary(), &db).await?;

        let config = RetrievalConfig::with_strategy(RetrievalStrategy::Default);
        let openai_client = Client::new();
        let params = pipeline::StrategyParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "tokio runtime worker behavior",
            user_id,
            config,
            reranker: None,
        };
        let results = pipeline::run_pipeline_with_embedding(params, test_embedding())
            .await?;

        let chunks = match results {
            StrategyOutput::Chunks(items) => items,
            other => anyhow::bail!("expected chunk results, got {other:?}"),
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
        Ok(())
    }

    #[tokio::test]
    async fn test_search_strategy_returns_search_result() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "search_user";
        let chunk = TextChunk::new(
            "search_src".into(),
            "Async Rust programming uses Tokio runtime for concurrent tasks.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(chunk.clone(), chunk_embedding_primary(), &db).await?;

        let config = RetrievalConfig::for_search(pipeline::SearchTarget::Both);
        let openai_client = Client::new();
        let params = pipeline::StrategyParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "async rust programming",
            user_id,
            config,
            reranker: None,
        };
        let results = pipeline::run_pipeline_with_embedding(params, test_embedding())
            .await?;

        let StrategyOutput::Search(search_result) = results else {
            anyhow::bail!("expected Search output");
        };

        // Should return chunks (entities may be empty if none stored)
        assert!(
            !search_result.chunks.is_empty(),
            "Search strategy should return chunks"
        );
        assert!(
            search_result
                .chunks
                .iter()
                .any(|c| c.chunk.chunk.contains("Tokio")),
            "Search results should contain relevant chunks"
        );
        Ok(())
    }
}
