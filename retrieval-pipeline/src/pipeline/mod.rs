mod config;
mod diagnostics;
mod stages;
mod strategies;

pub use config::{
    RetrievalConfig, RetrievalStrategy, RetrievalTuning, RetrievalTuningFlags, SearchTarget,
};
pub use diagnostics::{
    AssembleStats, ChunkEnrichmentStats, CollectCandidatesStats, EntityAssemblyTrace, Diagnostics,
};

use crate::{reranking::RerankerLease, RetrievedEntity, StrategyOutput};
use async_openai::Client;
use async_trait::async_trait;
use common::{error::AppError, storage::db::SurrealDbClient};
use std::time::{Duration, Instant};
use tracing::info;

use stages::PipelineContext;
use strategies::{
    DefaultStrategyDriver, IngestionDriver, RelationshipSuggestionDriver, SearchStrategyDriver,
};

// Export StrategyOutput publicly from this module
// (it's defined in lib.rs but we re-export it here)

// Stage type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageKind {
    Embed,
    CollectCandidates,
    GraphExpansion,
    ChunkAttach,
    Rerank,
    Assemble,
}

// Pipeline stage trait
#[async_trait]
pub trait Stage: Send + Sync {
    fn kind(&self) -> StageKind;
    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError>;
}

// Type alias for boxed stages
pub type BoxedStage = Box<dyn Stage>;

// Strategy driver trait
#[async_trait]
pub trait StrategyDriver: Send + Sync {
    type Output;

    fn stages(&self) -> Vec<BoxedStage>;
    fn finalize(&self, ctx: &mut PipelineContext<'_>) -> Result<Self::Output, Box<AppError>>;
}

// Pipeline stage timings tracker
#[derive(Debug, Default, Clone)]
pub struct StageTimings {
    timings: Vec<(StageKind, Duration)>,
}

impl StageTimings {
    pub fn record(&mut self, kind: StageKind, duration: Duration) {
        self.timings.push((kind, duration));
    }

    pub fn into_vec(self) -> Vec<(StageKind, Duration)> {
        self.timings
    }

    // Helper methods to get duration for each stage type (for backward compatibility)
    fn get_stage_ms(&self, kind: StageKind) -> u128 {
        self.timings
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(0, |(_, d)| d.as_millis())
    }

    pub fn embed_ms(&self) -> u128 {
        self.get_stage_ms(StageKind::Embed)
    }

    pub fn collect_candidates_ms(&self) -> u128 {
        self.get_stage_ms(StageKind::CollectCandidates)
    }

    pub fn graph_expansion_ms(&self) -> u128 {
        self.get_stage_ms(StageKind::GraphExpansion)
    }

    pub fn chunk_attach_ms(&self) -> u128 {
        self.get_stage_ms(StageKind::ChunkAttach)
    }

    pub fn rerank_ms(&self) -> u128 {
        self.get_stage_ms(StageKind::Rerank)
    }

    pub fn assemble_ms(&self) -> u128 {
        self.get_stage_ms(StageKind::Assemble)
    }
}

pub struct RunOutput<T> {
    pub results: T,
    pub diagnostics: Option<Diagnostics>,
    pub stage_timings: StageTimings,
}

pub async fn execute(params: StrategyParams<'_>) -> Result<StrategyOutput, AppError> {
    let input_chars = params.input_text.chars().count();
    let input_preview: String = params.input_text.chars().take(120).collect();
    let input_preview_clean = input_preview.replace('\n', " ");
    let preview_len = input_preview_clean.chars().count();
    info!(
        user_id = %params.user_id,
        input_chars,
        preview_truncated = input_chars > preview_len,
        preview = %input_preview_clean,
        strategy = %params.config.strategy,
        "Starting retrieval pipeline"
    );

    let strategy = params.config.strategy;
    let search_target = params.config.search_target;

    match strategy {
        RetrievalStrategy::Default => {
            let driver = DefaultStrategyDriver::new();
            let run = execute_strategy(driver, params, None, false).await?;
            Ok(StrategyOutput::Chunks(run.results))
        }
        RetrievalStrategy::RelationshipSuggestion => {
            let driver = RelationshipSuggestionDriver::new();
            let run = execute_strategy(driver, params, None, false).await?;
            Ok(StrategyOutput::Entities(run.results))
        }
        RetrievalStrategy::Ingestion => {
            let driver = IngestionDriver::new();
            let run = execute_strategy(driver, params, None, false).await?;
            Ok(StrategyOutput::Entities(run.results))
        }
        RetrievalStrategy::Search => {
            let driver = SearchStrategyDriver::new(search_target);
            let run = execute_strategy(driver, params, None, false).await?;
            Ok(StrategyOutput::Search(run.results))
        }
    }
}

pub async fn run_pipeline_with_embedding(
    params: StrategyParams<'_>,
    query_embedding: Vec<f32>,
) -> Result<StrategyOutput, AppError> {
    let strategy = params.config.strategy;
    let search_target = params.config.search_target;

    match strategy {
        RetrievalStrategy::Default => {
            let driver = DefaultStrategyDriver::new();
            let run = execute_strategy(driver, params, Some(query_embedding), false).await?;
            Ok(StrategyOutput::Chunks(run.results))
        }
        RetrievalStrategy::RelationshipSuggestion => {
            let driver = RelationshipSuggestionDriver::new();
            let run = execute_strategy(driver, params, Some(query_embedding), false).await?;
            Ok(StrategyOutput::Entities(run.results))
        }
        RetrievalStrategy::Ingestion => {
            let driver = IngestionDriver::new();
            let run = execute_strategy(driver, params, Some(query_embedding), false).await?;
            Ok(StrategyOutput::Entities(run.results))
        }
        RetrievalStrategy::Search => {
            let driver = SearchStrategyDriver::new(search_target);
            let run = execute_strategy(driver, params, Some(query_embedding), false).await?;
            Ok(StrategyOutput::Search(run.results))
        }
    }
}

pub async fn run_pipeline_with_embedding_with_metrics(
    params: StrategyParams<'_>,
    query_embedding: Vec<f32>,
) -> Result<RunOutput<StrategyOutput>, AppError> {
    let strategy = params.config.strategy;

    match strategy {
        RetrievalStrategy::Default => {
            let driver = DefaultStrategyDriver::new();
            let run = execute_strategy(driver, params, Some(query_embedding), false).await?;
            Ok(RunOutput {
                results: StrategyOutput::Chunks(run.results),
                diagnostics: run.diagnostics,
                stage_timings: run.stage_timings,
            })
        }
        _ => Err(AppError::InternalError(
            "Metrics not supported for this strategy".into(),
        )),
    }
}

pub async fn run_pipeline_with_embedding_with_diagnostics(
    params: StrategyParams<'_>,
    query_embedding: Vec<f32>,
) -> Result<RunOutput<StrategyOutput>, AppError> {
    let strategy = params.config.strategy;

    match strategy {
        RetrievalStrategy::Default => {
            let driver = DefaultStrategyDriver::new();
            let run = execute_strategy(driver, params, Some(query_embedding), true).await?;
            Ok(RunOutput {
                results: StrategyOutput::Chunks(run.results),
                diagnostics: run.diagnostics,
                stage_timings: run.stage_timings,
            })
        }
        _ => Err(AppError::InternalError(
            "Diagnostics not supported for this strategy".into(),
        )),
    }
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

pub struct StrategyParams<'a> {
    pub db_client: &'a SurrealDbClient,
    pub openai_client: &'a Client<async_openai::config::OpenAIConfig>,
    pub embedding_provider: Option<&'a common::utils::embedding::EmbeddingProvider>,
    pub input_text: &'a str,
    pub user_id: &'a str,
    pub config: RetrievalConfig,
    pub reranker: Option<RerankerLease>,
}

async fn execute_strategy<D: StrategyDriver>(
    driver: D,
    params: StrategyParams<'_>,
    query_embedding: Option<Vec<f32>>,
    capture_diagnostics: bool,
) -> Result<RunOutput<D::Output>, AppError> {
    let ctx = match query_embedding {
        Some(embedding) => PipelineContext::with_embedding(params, embedding),
        None => PipelineContext::new(params),
    };

    run_with_driver(driver, ctx, capture_diagnostics).await
}

async fn run_with_driver<D: StrategyDriver>(
    driver: D,
    mut ctx: PipelineContext<'_>,
    capture_diagnostics: bool,
) -> Result<RunOutput<D::Output>, AppError> {
    if capture_diagnostics {
        ctx.enable_diagnostics();
    }

    for stage in driver.stages() {
        let start = Instant::now();
        stage.execute(&mut ctx).await?;
        ctx.record_stage_duration(stage.kind(), start.elapsed());
    }

    let diagnostics = ctx.take_diagnostics();
    let stage_timings = ctx.take_stage_timings();
    let results = driver.finalize(&mut ctx).map_err(|e| *e)?;

    Ok(RunOutput {
        results,
        diagnostics,
        stage_timings,
    })
}

fn round_score(value: f32) -> f64 {
    (f64::from(value) * 1000.0).round() / 1000.0
}
