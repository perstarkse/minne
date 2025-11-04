mod config;
mod diagnostics;
mod stages;
mod state;

pub use config::{RetrievalConfig, RetrievalTuning};
pub use diagnostics::{
    AssembleStats, ChunkEnrichmentStats, CollectCandidatesStats, EntityAssemblyTrace,
    PipelineDiagnostics,
};

use crate::{reranking::RerankerLease, RetrievedEntity};
use async_openai::Client;
use common::{error::AppError, storage::db::SurrealDbClient};
use tracing::info;

#[derive(Debug)]
pub struct PipelineRunOutput {
    pub results: Vec<RetrievedEntity>,
    pub diagnostics: Option<PipelineDiagnostics>,
    pub stage_timings: PipelineStageTimings,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PipelineStageTimings {
    pub collect_candidates_ms: u128,
    pub graph_expansion_ms: u128,
    pub chunk_attach_ms: u128,
    pub rerank_ms: u128,
    pub assemble_ms: u128,
}

impl PipelineStageTimings {
    fn record_collect_candidates(&mut self, duration: std::time::Duration) {
        self.collect_candidates_ms += duration.as_millis() as u128;
    }

    fn record_graph_expansion(&mut self, duration: std::time::Duration) {
        self.graph_expansion_ms += duration.as_millis() as u128;
    }

    fn record_chunk_attach(&mut self, duration: std::time::Duration) {
        self.chunk_attach_ms += duration.as_millis() as u128;
    }

    fn record_rerank(&mut self, duration: std::time::Duration) {
        self.rerank_ms += duration.as_millis() as u128;
    }

    fn record_assemble(&mut self, duration: std::time::Duration) {
        self.assemble_ms += duration.as_millis() as u128;
    }
}

/// Drives the retrieval pipeline from embedding through final assembly.
pub async fn run_pipeline(
    db_client: &SurrealDbClient,
    openai_client: &Client<async_openai::config::OpenAIConfig>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<Vec<RetrievedEntity>, AppError> {
    let input_chars = input_text.chars().count();
    let input_preview: String = input_text.chars().take(120).collect();
    let input_preview_clean = input_preview.replace('\n', " ");
    let preview_len = input_preview_clean.chars().count();
    info!(
        %user_id,
        input_chars,
        preview_truncated = input_chars > preview_len,
        preview = %input_preview_clean,
        "Starting ingestion retrieval pipeline"
    );
    let ctx = stages::PipelineContext::new(
        db_client,
        openai_client,
        input_text.to_owned(),
        user_id.to_owned(),
        config,
        reranker,
    );
    let outcome = run_pipeline_internal(ctx, false).await?;

    Ok(outcome.results)
}

pub async fn run_pipeline_with_embedding(
    db_client: &SurrealDbClient,
    openai_client: &Client<async_openai::config::OpenAIConfig>,
    query_embedding: Vec<f32>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<Vec<RetrievedEntity>, AppError> {
    let ctx = stages::PipelineContext::with_embedding(
        db_client,
        openai_client,
        query_embedding,
        input_text.to_owned(),
        user_id.to_owned(),
        config,
        reranker,
    );
    let outcome = run_pipeline_internal(ctx, false).await?;

    Ok(outcome.results)
}

/// Runs the pipeline with a precomputed embedding and returns stage metrics.
pub async fn run_pipeline_with_embedding_with_metrics(
    db_client: &SurrealDbClient,
    openai_client: &Client<async_openai::config::OpenAIConfig>,
    query_embedding: Vec<f32>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<PipelineRunOutput, AppError> {
    let ctx = stages::PipelineContext::with_embedding(
        db_client,
        openai_client,
        query_embedding,
        input_text.to_owned(),
        user_id.to_owned(),
        config,
        reranker,
    );

    run_pipeline_internal(ctx, false).await
}

pub async fn run_pipeline_with_embedding_with_diagnostics(
    db_client: &SurrealDbClient,
    openai_client: &Client<async_openai::config::OpenAIConfig>,
    query_embedding: Vec<f32>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
    reranker: Option<RerankerLease>,
) -> Result<PipelineRunOutput, AppError> {
    let ctx = stages::PipelineContext::with_embedding(
        db_client,
        openai_client,
        query_embedding,
        input_text.to_owned(),
        user_id.to_owned(),
        config,
        reranker,
    );

    run_pipeline_internal(ctx, true).await
}

/// Helper exposed for tests to convert retrieved entities into downstream prompt JSON.
pub fn retrieved_entities_to_json(entities: &[RetrievedEntity]) -> serde_json::Value {
    serde_json::json!(entities
        .iter()
        .map(|entry| {
            serde_json::json!({
                "KnowledgeEntity": {
                    "id": entry.entity.id,
                    "name": entry.entity.name,
                    "description": entry.entity.description,
                    "score": round_score(entry.score),
                    "chunks": entry.chunks.iter().map(|chunk| {
                        serde_json::json!({
                            "score": round_score(chunk.score),
                            "content": chunk.chunk.chunk
                        })
                    }).collect::<Vec<_>>()
                }
            })
        })
        .collect::<Vec<_>>())
}

async fn run_pipeline_internal(
    mut ctx: stages::PipelineContext<'_>,
    capture_diagnostics: bool,
) -> Result<PipelineRunOutput, AppError> {
    if capture_diagnostics {
        ctx.enable_diagnostics();
    }

    let results = drive_pipeline(&mut ctx).await?;
    let diagnostics = ctx.take_diagnostics();

    Ok(PipelineRunOutput {
        results,
        diagnostics,
        stage_timings: ctx.take_stage_timings(),
    })
}

async fn drive_pipeline(
    ctx: &mut stages::PipelineContext<'_>,
) -> Result<Vec<RetrievedEntity>, AppError> {
    let machine = state::ready();
    let machine = stages::embed(machine, ctx).await?;
    let machine = stages::collect_candidates(machine, ctx).await?;
    let machine = stages::expand_graph(machine, ctx).await?;
    let machine = stages::attach_chunks(machine, ctx).await?;
    let machine = stages::rerank(machine, ctx).await?;
    let results = stages::assemble(machine, ctx)?;
    Ok(results)
}

fn round_score(value: f32) -> f64 {
    (f64::from(value) * 1000.0).round() / 1000.0
}
