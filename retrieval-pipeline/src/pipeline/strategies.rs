use super::{
    stages::{
        AssembleEntitiesStage, ChunkAssembleStage, ChunkRerankStage, ChunkVectorStage,
        CollectCandidatesStage, EmbedStage, GraphExpansionStage, PipelineContext, RerankStage,
    },
    BoxedStage, StrategyDriver,
};
use crate::{RetrievedChunk, RetrievedEntity};
use common::error::AppError;



pub struct DefaultStrategyDriver;

impl DefaultStrategyDriver {
    pub fn new() -> Self {
        Self
    }
}

impl StrategyDriver for DefaultStrategyDriver {
    type Output = Vec<RetrievedChunk>;

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
}

pub struct RelationshipSuggestionDriver;

impl RelationshipSuggestionDriver {
    pub fn new() -> Self {
        Self
    }
}

impl StrategyDriver for RelationshipSuggestionDriver {
    type Output = Vec<RetrievedEntity>;

    fn stages(&self) -> Vec<BoxedStage> {
        vec![
            Box::new(EmbedStage),
            Box::new(CollectCandidatesStage),
            Box::new(GraphExpansionStage),
            // Skip ChunkAttachStage
            Box::new(RerankStage),
            Box::new(AssembleEntitiesStage),
        ]
    }

    fn finalize(&self, ctx: &mut PipelineContext<'_>) -> Result<Self::Output, AppError> {
        Ok(ctx.take_entity_results())
    }
}

pub struct IngestionDriver;

impl IngestionDriver {
    pub fn new() -> Self {
        Self
    }
}

impl StrategyDriver for IngestionDriver {
    type Output = Vec<RetrievedEntity>;

    fn stages(&self) -> Vec<BoxedStage> {
        vec![
            Box::new(EmbedStage),
            Box::new(CollectCandidatesStage),
            Box::new(GraphExpansionStage),
            // Skip ChunkAttachStage
            Box::new(RerankStage),
            Box::new(AssembleEntitiesStage),
        ]
    }

    fn finalize(&self, ctx: &mut PipelineContext<'_>) -> Result<Self::Output, AppError> {
        Ok(ctx.take_entity_results())
    }
}
