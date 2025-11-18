use super::{
    stages::{
        AssembleEntitiesStage, ChunkAssembleStage, ChunkAttachStage, ChunkRerankStage,
        ChunkVectorStage, CollectCandidatesStage, EmbedStage, GraphExpansionStage, PipelineContext,
        RerankStage,
    },
    BoxedStage, RetrievalConfig, RetrievalStrategy, StrategyDriver,
};
use crate::{RetrievedChunk, RetrievedEntity};
use common::error::AppError;

pub struct InitialStrategyDriver;

impl InitialStrategyDriver {
    pub fn new() -> Self {
        Self
    }
}

impl StrategyDriver for InitialStrategyDriver {
    type Output = Vec<RetrievedEntity>;

    fn strategy(&self) -> RetrievalStrategy {
        RetrievalStrategy::Initial
    }

    fn stages(&self) -> Vec<BoxedStage> {
        vec![
            Box::new(EmbedStage),
            Box::new(CollectCandidatesStage),
            Box::new(GraphExpansionStage),
            Box::new(ChunkAttachStage),
            Box::new(RerankStage),
            Box::new(AssembleEntitiesStage),
        ]
    }

    fn finalize(&self, ctx: &mut PipelineContext<'_>) -> Result<Self::Output, AppError> {
        Ok(ctx.take_entity_results())
    }
}

pub struct RevisedStrategyDriver;

impl RevisedStrategyDriver {
    pub fn new() -> Self {
        Self
    }
}

impl StrategyDriver for RevisedStrategyDriver {
    type Output = Vec<RetrievedChunk>;

    fn strategy(&self) -> RetrievalStrategy {
        RetrievalStrategy::Revised
    }

    fn stages(&self) -> Vec<BoxedStage> {
        vec![
            Box::new(EmbedStage),
            Box::new(ChunkVectorStage),
            Box::new(ChunkRerankStage),
            Box::new(ChunkAssembleStage),
        ]
    }

    fn finalize(&self, ctx: &mut PipelineContext<'_>) -> Result<Self::Output, AppError> {
        Ok(ctx.take_chunk_results())
    }

    fn override_tuning(&self, config: &mut RetrievalConfig) {
        config.tuning.entity_vector_take = 0;
        config.tuning.entity_fts_take = 0;
    }
}
