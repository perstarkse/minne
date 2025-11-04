use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use async_openai::Client;
use common::storage::{
    db::SurrealDbClient,
    types::{system_settings::SystemSettings, user::User},
};
use composite_retrieval::{
    pipeline::{PipelineStageTimings, RetrievalConfig},
    reranking::RerankerPool,
};

use crate::{
    args::Config,
    cache::EmbeddingCache,
    datasets::ConvertedDataset,
    embedding::EmbeddingProvider,
    eval::{CaseDiagnostics, CaseSummary, EvaluationStageTimings, EvaluationSummary, SeededCase},
    ingest, slice, snapshot,
};

pub(super) struct EvaluationContext<'a> {
    dataset: &'a ConvertedDataset,
    config: &'a Config,
    pub stage_timings: EvaluationStageTimings,
    pub ledger_limit: Option<usize>,
    pub slice_settings: Option<slice::SliceConfig<'a>>,
    pub slice: Option<slice::ResolvedSlice<'a>>,
    pub window_offset: usize,
    pub window_length: usize,
    pub window_total_cases: usize,
    pub namespace: String,
    pub database: String,
    pub db: Option<SurrealDbClient>,
    pub descriptor: Option<snapshot::Descriptor>,
    pub settings: Option<SystemSettings>,
    pub settings_missing: bool,
    pub must_reapply_settings: bool,
    pub embedding_provider: Option<EmbeddingProvider>,
    pub embedding_cache: Option<EmbeddingCache>,
    pub openai_client: Option<Arc<Client<async_openai::config::OpenAIConfig>>>,
    pub openai_base_url: Option<String>,
    pub expected_fingerprint: Option<String>,
    pub ingestion_duration_ms: u128,
    pub namespace_seed_ms: Option<u128>,
    pub namespace_reused: bool,
    pub evaluation_start: Option<Instant>,
    pub eval_user: Option<User>,
    pub corpus_handle: Option<ingest::CorpusHandle>,
    pub cases: Vec<SeededCase>,
    pub stage_latency_samples: Vec<PipelineStageTimings>,
    pub latencies: Vec<u128>,
    pub diagnostics_output: Vec<CaseDiagnostics>,
    pub query_summaries: Vec<CaseSummary>,
    pub rerank_pool: Option<Arc<RerankerPool>>,
    pub retrieval_config: Option<Arc<RetrievalConfig>>,
    pub summary: Option<EvaluationSummary>,
    pub diagnostics_path: Option<PathBuf>,
    pub diagnostics_enabled: bool,
}

impl<'a> EvaluationContext<'a> {
    pub fn new(dataset: &'a ConvertedDataset, config: &'a Config) -> Self {
        Self {
            dataset,
            config,
            stage_timings: EvaluationStageTimings::default(),
            ledger_limit: None,
            slice_settings: None,
            slice: None,
            window_offset: 0,
            window_length: 0,
            window_total_cases: 0,
            namespace: String::new(),
            database: String::new(),
            db: None,
            descriptor: None,
            settings: None,
            settings_missing: false,
            must_reapply_settings: false,
            embedding_provider: None,
            embedding_cache: None,
            openai_client: None,
            openai_base_url: None,
            expected_fingerprint: None,
            ingestion_duration_ms: 0,
            namespace_seed_ms: None,
            namespace_reused: false,
            evaluation_start: None,
            eval_user: None,
            corpus_handle: None,
            cases: Vec::new(),
            stage_latency_samples: Vec::new(),
            latencies: Vec::new(),
            diagnostics_output: Vec::new(),
            query_summaries: Vec::new(),
            rerank_pool: None,
            retrieval_config: None,
            summary: None,
            diagnostics_path: config.chunk_diagnostics_path.clone(),
            diagnostics_enabled: config.chunk_diagnostics_path.is_some(),
        }
    }

    pub fn dataset(&self) -> &'a ConvertedDataset {
        self.dataset
    }

    pub fn config(&self) -> &'a Config {
        self.config
    }

    pub fn slice(&self) -> &slice::ResolvedSlice<'a> {
        self.slice.as_ref().expect("slice has not been prepared")
    }

    pub fn db(&self) -> &SurrealDbClient {
        self.db.as_ref().expect("database connection missing")
    }

    pub fn descriptor(&self) -> &snapshot::Descriptor {
        self.descriptor
            .as_ref()
            .expect("snapshot descriptor unavailable")
    }

    pub fn embedding_provider(&self) -> &EmbeddingProvider {
        self.embedding_provider
            .as_ref()
            .expect("embedding provider not initialised")
    }

    pub fn openai_client(&self) -> Arc<Client<async_openai::config::OpenAIConfig>> {
        self.openai_client
            .as_ref()
            .expect("openai client missing")
            .clone()
    }

    pub fn corpus_handle(&self) -> &ingest::CorpusHandle {
        self.corpus_handle.as_ref().expect("corpus handle missing")
    }

    pub fn evaluation_user(&self) -> &User {
        self.eval_user.as_ref().expect("evaluation user missing")
    }

    pub fn record_stage_duration(&mut self, stage: EvalStage, duration: Duration) {
        let elapsed = duration.as_millis() as u128;
        match stage {
            EvalStage::PrepareSlice => self.stage_timings.prepare_slice_ms += elapsed,
            EvalStage::PrepareDb => self.stage_timings.prepare_db_ms += elapsed,
            EvalStage::PrepareCorpus => self.stage_timings.prepare_corpus_ms += elapsed,
            EvalStage::PrepareNamespace => self.stage_timings.prepare_namespace_ms += elapsed,
            EvalStage::RunQueries => self.stage_timings.run_queries_ms += elapsed,
            EvalStage::Summarize => self.stage_timings.summarize_ms += elapsed,
            EvalStage::Finalize => self.stage_timings.finalize_ms += elapsed,
        }
    }

    pub fn into_summary(self) -> EvaluationSummary {
        self.summary.expect("evaluation summary missing")
    }
}

#[derive(Copy, Clone)]
pub(super) enum EvalStage {
    PrepareSlice,
    PrepareDb,
    PrepareCorpus,
    PrepareNamespace,
    RunQueries,
    Summarize,
    Finalize,
}

impl EvalStage {
    pub fn label(&self) -> &'static str {
        match self {
            EvalStage::PrepareSlice => "prepare-slice",
            EvalStage::PrepareDb => "prepare-db",
            EvalStage::PrepareCorpus => "prepare-corpus",
            EvalStage::PrepareNamespace => "prepare-namespace",
            EvalStage::RunQueries => "run-queries",
            EvalStage::Summarize => "summarize",
            EvalStage::Finalize => "finalize",
        }
    }
}
