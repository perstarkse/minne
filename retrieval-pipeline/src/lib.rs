pub mod answer_retrieval;

pub mod pipeline;
pub mod reranking;

pub(crate) mod scoring;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk},
    },
};
use reranking::RerankerLease;
use tracing::instrument;

/// Result of a retrieval run.
///
/// Chunk retrieval is always performed; entities are only present when the caller
/// requested entity resolution via [`RetrievalConfig::with_entities`].
#[derive(Debug)]
pub enum RetrievalOutput {
    Chunks(Vec<RetrievedChunk>),
    WithEntities {
        chunks: Vec<RetrievedChunk>,
        entities: Vec<RetrievedEntity>,
    },
}

pub use pipeline::{
    retrieved_entities_to_json, Diagnostics, RetrievalConfig, RetrievalParams, StageKind,
    StageTimings,
};

/// Round a score to three decimal places for JSON output.
pub(crate) fn round_score(value: f32) -> f64 {
    (f64::from(value) * 1000.0).round() / 1000.0
}

// Captures a supporting chunk plus its fused retrieval score for downstream prompts.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub chunk: TextChunk,
    pub score: f32,
}

// Knowledge entity resolved from retrieved chunks, enriched with its contributing chunks.
#[derive(Debug, Clone)]
pub struct RetrievedEntity {
    pub entity: KnowledgeEntity,
    pub score: f32,
    pub chunks: Vec<RetrievedChunk>,
}

/// Run chunk-first hybrid retrieval for `input_text`, optionally resolving owning entities.
#[instrument(skip_all, fields(user_id))]
pub async fn retrieve(
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    embedding_provider: Option<&common::utils::embedding::EmbeddingProvider>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<RetrievalOutput, AppError> {
    let params = pipeline::RetrievalParams {
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
    use common::storage::types::knowledge_entity::{KnowledgeEntity, KnowledgeEntityType};
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
    async fn test_chunk_retrieval_returns_chunks() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "test_user";
        let chunk = TextChunk::new(
            "source_1".into(),
            "Tokio uses cooperative scheduling for fairness.".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(chunk.clone(), chunk_embedding_primary(), &db).await?;

        let openai_client = Client::new();
        let params = pipeline::RetrievalParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "Rust concurrency async tasks",
            user_id,
            config: RetrievalConfig::default(),
            reranker: None,
        };
        let results = pipeline::run_with_embedding(params, test_embedding()).await?;

        let chunks = match results {
            RetrievalOutput::Chunks(items) => items,
            RetrievalOutput::WithEntities { .. } => {
                anyhow::bail!("expected chunk results, got entities")
            }
        };

        assert!(!chunks.is_empty(), "Expected at least one retrieval result");
        assert!(
            chunks.first().is_some_and(|c| c.chunk.chunk.contains("Tokio")),
            "Expected chunk about Tokio"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_chunk_retrieval_returns_chunks_from_multiple_sources() -> anyhow::Result<()> {
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
        let params = pipeline::RetrievalParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "Rust concurrency async tasks",
            user_id,
            config: RetrievalConfig::default(),
            reranker: None,
        };
        let results = pipeline::run_with_embedding(params, test_embedding()).await?;

        let chunks = match results {
            RetrievalOutput::Chunks(items) => items,
            RetrievalOutput::WithEntities { .. } => {
                anyhow::bail!("expected chunk results, got entities")
            }
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
    async fn test_with_entities_resolves_owning_entities() -> anyhow::Result<()> {
        let db = setup_test_db().await?;
        let user_id = "entity_user";

        let chunk = TextChunk::new(
            "entity_source".into(),
            "Async Rust programming uses the Tokio runtime for concurrent tasks.".into(),
            user_id.into(),
        );
        TextChunk::store_with_embedding(chunk.clone(), chunk_embedding_primary(), &db).await?;

        let entity = KnowledgeEntity::new(
            "entity_source".into(),
            "Tokio Runtime".into(),
            "Async runtime for Rust".into(),
            KnowledgeEntityType::Document,
            None,
            user_id.into(),
        );
        db.store_item(entity).await?;

        let openai_client = Client::new();
        let params = pipeline::RetrievalParams {
            db_client: &db,
            openai_client: &openai_client,
            embedding_provider: None,
            input_text: "async rust programming",
            user_id,
            config: RetrievalConfig::with_entities(),
            reranker: None,
        };
        let results = pipeline::run_with_embedding(params, test_embedding()).await?;

        let RetrievalOutput::WithEntities { chunks, entities } = results else {
            anyhow::bail!("expected WithEntities output");
        };

        assert!(!chunks.is_empty(), "Should return chunks");
        assert!(
            entities.iter().any(|e| e.entity.name == "Tokio Runtime"),
            "Should resolve the entity owning the retrieved chunk"
        );
        assert!(
            entities
                .iter()
                .find(|e| e.entity.name == "Tokio Runtime")
                .is_some_and(|e| !e.chunks.is_empty()),
            "Resolved entity should carry its contributing chunks"
        );
        Ok(())
    }
}
