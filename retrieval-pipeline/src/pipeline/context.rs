use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::text_chunk::TextChunk},
    utils::embedding::EmbeddingProvider,
};

use crate::scoring::Scored;

use crate::{reranking::RerankerLease, RetrievedChunk, RetrievedEntity};

use super::{
    config::RetrievalConfig,
    diagnostics::{AssembleStats, Diagnostics, SearchStats},
    StageKind, StageTimings, RetrievalParams,
};

/// Mutable working state threaded through every retrieval stage.
pub(crate) struct PipelineContext<'a> {
    pub db_client: &'a SurrealDbClient,
    pub embedding_provider: &'a EmbeddingProvider,
    pub input_text: String,
    pub user_id: String,
    pub config: RetrievalConfig,
    pub query_embedding: Option<Vec<f32>>,
    pub chunk_values: Vec<Scored<TextChunk>>,
    pub reranker: Option<RerankerLease>,
    pub diagnostics: Option<Diagnostics>,
    pub entity_results: Vec<RetrievedEntity>,
    pub chunk_results: Vec<RetrievedChunk>,
    stage_timings: StageTimings,
}

impl<'a> PipelineContext<'a> {
    pub fn new(params: RetrievalParams<'a>) -> Self {
        Self {
            db_client: params.db_client,
            embedding_provider: params.embedding_provider,
            input_text: params.input_text.to_owned(),
            user_id: params.user_id.to_owned(),
            config: params.config,
            query_embedding: None,
            chunk_values: Vec::new(),
            reranker: params.reranker,
            diagnostics: None,
            entity_results: Vec::new(),
            chunk_results: Vec::new(),
            stage_timings: StageTimings::default(),
        }
    }

    pub fn with_embedding(params: RetrievalParams<'a>, query_embedding: Vec<f32>) -> Self {
        let mut ctx = Self::new(params);
        ctx.query_embedding = Some(query_embedding);
        ctx
    }

    pub(crate) fn ensure_embedding(&self) -> Result<&Vec<f32>, Box<AppError>> {
        self.query_embedding.as_ref().ok_or_else(|| {
            Box::new(AppError::InternalError(
                "query embedding missing before candidate search".to_string(),
            ))
        })
    }

    pub fn enable_diagnostics(&mut self) {
        if self.diagnostics.is_none() {
            self.diagnostics = Some(Diagnostics::default());
        }
    }

    pub fn diagnostics_enabled(&self) -> bool {
        self.diagnostics.is_some()
    }

    pub(crate) fn record_search(&mut self, stats: SearchStats) {
        if let Some(diag) = self.diagnostics.as_mut() {
            diag.search = Some(stats);
        }
    }

    pub(crate) fn record_assemble(&mut self, stats: AssembleStats) {
        if let Some(diag) = self.diagnostics.as_mut() {
            diag.assemble = Some(stats);
        }
    }

    pub fn take_diagnostics(&mut self) -> Option<Diagnostics> {
        self.diagnostics.take()
    }

    pub fn take_stage_timings(&mut self) -> StageTimings {
        std::mem::take(&mut self.stage_timings)
    }

    pub fn record_stage_duration(&mut self, kind: StageKind, duration: std::time::Duration) {
        self.stage_timings.record(kind, duration);
    }

    pub fn take_entity_results(&mut self) -> Vec<RetrievedEntity> {
        std::mem::take(&mut self.entity_results)
    }

    pub fn take_chunk_results(&mut self) -> Vec<RetrievedChunk> {
        std::mem::take(&mut self.chunk_results)
    }
}
