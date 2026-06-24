use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use async_openai::Client;
use common::{
    storage::{
        db::SurrealDbClient,
        types::{system_settings::SystemSettings, user::User},
    },
    utils::embedding::EmbeddingProvider,
};
use retrieval_pipeline::{
    pipeline::{RetrievalConfig, StageTimings},
    reranking::RerankerPool,
};

use crate::{
    args::Config,
    cases::SeededCase,
    corpus,
    datasets::ConvertedDataset,
    slice,
    types::{CaseDiagnostics, CaseSummary, EvaluationStageTimings, EvaluationSummary},
};

#[allow(clippy::struct_excessive_bools)]
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
    pub settings: Option<SystemSettings>,
    pub settings_missing: bool,
    pub must_reapply_settings: bool,
    pub embedding_provider: Option<EmbeddingProvider>,
    pub openai_client: Option<Arc<Client<async_openai::config::OpenAIConfig>>>,
    pub openai_base_url: Option<String>,
    pub expected_fingerprint: Option<String>,
    pub ingestion_duration_ms: u128,
    pub namespace_seed_ms: Option<u128>,
    pub namespace_reused: bool,
    pub evaluation_start: Option<Instant>,
    pub eval_user: Option<User>,
    pub corpus_handle: Option<corpus::CorpusHandle>,
    pub cases: Vec<SeededCase>,
    pub filtered_questions: usize,
    pub stage_latency_samples: Vec<StageTimings>,
    pub latencies: Vec<u128>,
    pub diagnostics_output: Vec<CaseDiagnostics>,
    pub query_summaries: Vec<CaseSummary>,
    pub rerank_pool: Option<Arc<RerankerPool>>,
    pub retrieval_config: Option<Arc<RetrievalConfig>>,
    pub summary: Option<EvaluationSummary>,
    pub diagnostics_path: Option<PathBuf>,
    pub diagnostics_enabled: bool,
    pub content_checksum: Option<String>,
}

impl<'a> EvaluationContext<'a> {
    pub fn new(
        dataset: &'a ConvertedDataset,
        config: &'a Config,
        content_checksum: Option<String>,
    ) -> Self {
        Self {
            dataset,
            config,
            content_checksum,
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
            settings: None,
            settings_missing: false,
            must_reapply_settings: false,
            embedding_provider: None,
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
            filtered_questions: 0,
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

    pub fn slice(&self) -> Result<&slice::ResolvedSlice<'a>> {
        self.slice
            .as_ref()
            .ok_or_else(|| anyhow!("slice has not been prepared"))
    }

    pub fn db(&self) -> Result<&SurrealDbClient> {
        self.db
            .as_ref()
            .ok_or_else(|| anyhow!("database connection missing"))
    }

    pub fn embedding_provider(&self) -> Result<&EmbeddingProvider> {
        self.embedding_provider
            .as_ref()
            .ok_or_else(|| anyhow!("embedding provider not initialised"))
    }

    pub fn openai_client(&self) -> Result<Arc<Client<async_openai::config::OpenAIConfig>>> {
        Ok(Arc::clone(
            self.openai_client
                .as_ref()
                .ok_or_else(|| anyhow!("openai client missing"))?,
        ))
    }

    pub fn corpus_handle(&self) -> Result<&corpus::CorpusHandle> {
        self.corpus_handle
            .as_ref()
            .ok_or_else(|| anyhow!("corpus handle missing"))
    }

    pub fn content_checksum(&self) -> Option<&str> {
        self.content_checksum.as_deref()
    }

    pub fn evaluation_user(&self) -> Result<&User> {
        self.eval_user
            .as_ref()
            .ok_or_else(|| anyhow!("evaluation user missing"))
    }

    #[allow(clippy::arithmetic_side_effects)]
    pub fn record_stage_duration(&mut self, stage: EvalStage, duration: Duration) {
        let elapsed = duration.as_millis();
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

    pub fn into_summary(self) -> Result<EvaluationSummary> {
        self.summary
            .ok_or_else(|| anyhow!("evaluation summary missing"))
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
    pub fn label(self) -> &'static str {
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
