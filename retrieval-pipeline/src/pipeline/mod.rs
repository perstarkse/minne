mod config;
mod context;
mod diagnostics;
mod stages;

pub use config::{RetrievalConfig, RetrievalTuning};
pub use diagnostics::Diagnostics;

use crate::{round_score, RetrievalOutput, RetrievedEntity};
use async_openai::Client;
use async_trait::async_trait;
use common::{error::AppError, storage::db::SurrealDbClient};
use std::time::{Duration, Instant};
use tracing::info;

use stages::{
    ChunkAssembleStage, ChunkRerankStage, ChunkSearchStage, EmbedStage, ResolveEntitiesStage,
};

/// Identifies a retrieval stage for timing and instrumentation.
///
/// [`StageKind::ALL`] lists every kind in pipeline order; consumers (e.g. the evaluation
/// harness) iterate it generically so that adding a stage requires no changes outside this
/// crate — add the variant, extend `ALL`, and give it a [`StageKind::label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageKind {
    Embed,
    Search,
    Rerank,
    ResolveEntities,
    Assemble,
}

impl StageKind {
    /// Every stage kind in canonical pipeline order.
    pub const ALL: [StageKind; 5] = [
        StageKind::Embed,
        StageKind::Search,
        StageKind::Rerank,
        StageKind::ResolveEntities,
        StageKind::Assemble,
    ];

    /// Stable, machine-friendly identifier for the stage (used as a metrics key).
    pub const fn label(self) -> &'static str {
        match self {
            StageKind::Embed => "embed",
            StageKind::Search => "search",
            StageKind::Rerank => "rerank",
            StageKind::ResolveEntities => "resolve_entities",
            StageKind::Assemble => "assemble",
        }
    }
}

/// A single composable step in the retrieval pipeline.
#[async_trait]
pub(crate) trait Stage: Send + Sync {
    fn kind(&self) -> StageKind;
    async fn execute(&self, ctx: &mut context::PipelineContext<'_>) -> Result<(), AppError>;
}

pub(crate) type BoxedStage = Box<dyn Stage>;

/// Per-stage execution timings recorded during a run.
#[derive(Debug, Default, Clone)]
pub struct StageTimings {
    timings: Vec<(StageKind, Duration)>,
}

impl StageTimings {
    pub fn record(&mut self, kind: StageKind, duration: Duration) {
        self.timings.push((kind, duration));
    }

    /// Milliseconds recorded for `kind`, or `0` if the stage did not run.
    pub fn stage_ms(&self, kind: StageKind) -> u128 {
        self.timings
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(0, |(_, d)| d.as_millis())
    }
}

pub struct RunOutput<T> {
    pub results: T,
    pub diagnostics: Option<Diagnostics>,
    pub stage_timings: StageTimings,
}

/// Inputs required to run a retrieval.
pub struct RetrievalParams<'a> {
    pub db_client: &'a SurrealDbClient,
    pub openai_client: &'a Client<async_openai::config::OpenAIConfig>,
    pub embedding_provider: Option<&'a common::utils::embedding::EmbeddingProvider>,
    pub input_text: &'a str,
    pub user_id: &'a str,
    pub config: RetrievalConfig,
    pub reranker: Option<crate::reranking::RerankerLease>,
}

fn build_stages(config: &RetrievalConfig) -> Vec<BoxedStage> {
    let mut stages: Vec<BoxedStage> = vec![
        Box::new(EmbedStage),
        Box::new(ChunkSearchStage),
        Box::new(ChunkRerankStage),
    ];
    if config.resolve_entities {
        stages.push(Box::new(ResolveEntitiesStage));
    }
    stages.push(Box::new(ChunkAssembleStage));
    stages
}

async fn run(
    params: RetrievalParams<'_>,
    query_embedding: Option<Vec<f32>>,
    capture_diagnostics: bool,
) -> Result<RunOutput<RetrievalOutput>, AppError> {
    let input_chars = params.input_text.chars().count();
    let input_preview: String = params.input_text.chars().take(120).collect();
    let input_preview_clean = input_preview.replace('\n', " ");
    let preview_len = input_preview_clean.chars().count();
    info!(
        user_id = %params.user_id,
        input_chars,
        preview_truncated = input_chars > preview_len,
        preview = %input_preview_clean,
        resolve_entities = params.config.resolve_entities,
        "Starting retrieval pipeline"
    );

    let resolve_entities = params.config.resolve_entities;
    let mut ctx = match query_embedding {
        Some(embedding) => context::PipelineContext::with_embedding(params, embedding),
        None => context::PipelineContext::new(params),
    };

    if capture_diagnostics {
        ctx.enable_diagnostics();
    }

    for stage in build_stages(&ctx.config) {
        let start = Instant::now();
        stage.execute(&mut ctx).await?;
        ctx.record_stage_duration(stage.kind(), start.elapsed());
    }

    let diagnostics = ctx.take_diagnostics();
    let stage_timings = ctx.take_stage_timings();
    let chunks = ctx.take_chunk_results();
    let results = if resolve_entities {
        RetrievalOutput::WithEntities {
            chunks,
            entities: ctx.take_entity_results(),
        }
    } else {
        RetrievalOutput::Chunks(chunks)
    };

    Ok(RunOutput {
        results,
        diagnostics,
        stage_timings,
    })
}

/// Run the retrieval pipeline, generating the query embedding internally if needed.
pub async fn execute(params: RetrievalParams<'_>) -> Result<RetrievalOutput, AppError> {
    Ok(run(params, None, false).await?.results)
}

/// Run the retrieval pipeline with a pre-computed query embedding.
pub async fn run_with_embedding(
    params: RetrievalParams<'_>,
    query_embedding: Vec<f32>,
) -> Result<RetrievalOutput, AppError> {
    Ok(run(params, Some(query_embedding), false).await?.results)
}

/// Run with a pre-computed embedding, returning results and per-stage timings.
///
/// When `capture_diagnostics` is true, pipeline search/assemble stats are included.
pub async fn run_with_embedding_instrumented(
    params: RetrievalParams<'_>,
    query_embedding: Vec<f32>,
    capture_diagnostics: bool,
) -> Result<RunOutput<RetrievalOutput>, AppError> {
    run(params, Some(query_embedding), capture_diagnostics).await
}

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
