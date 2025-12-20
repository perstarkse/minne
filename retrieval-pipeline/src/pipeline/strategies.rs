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

use crate::SearchResult;
use super::config::SearchTarget;

/// Search strategy driver that retrieves both chunks and entities
pub struct SearchStrategyDriver {
    target: SearchTarget,
}

impl SearchStrategyDriver {
    pub fn new(target: SearchTarget) -> Self {
        Self { target }
    }
}

impl StrategyDriver for SearchStrategyDriver {
    type Output = SearchResult;

    fn stages(&self) -> Vec<BoxedStage> {
        match self.target {
            SearchTarget::ChunksOnly => vec![
                Box::new(EmbedStage),
                Box::new(ChunkVectorStage),
                Box::new(ChunkRerankStage),
                Box::new(ChunkAssembleStage),
            ],
            SearchTarget::EntitiesOnly => vec![
                Box::new(EmbedStage),
                Box::new(CollectCandidatesStage),
                Box::new(GraphExpansionStage),
                Box::new(RerankStage),
                Box::new(AssembleEntitiesStage),
            ],
            SearchTarget::Both => vec![
                Box::new(EmbedStage),
                // Chunk retrieval path
                Box::new(ChunkVectorStage),
                Box::new(ChunkRerankStage),
                Box::new(ChunkAssembleStage),
                // Entity retrieval path (runs after chunk stages)
                Box::new(CollectCandidatesStage),
                Box::new(GraphExpansionStage),
                Box::new(RerankStage),
                Box::new(AssembleEntitiesStage),
            ],
        }
    }

    fn finalize(&self, ctx: &mut PipelineContext<'_>) -> Result<Self::Output, AppError> {
        let chunks = match self.target {
            SearchTarget::EntitiesOnly => Vec::new(),
            _ => ctx.take_chunk_results(),
        };
        let entities = match self.target {
            SearchTarget::ChunksOnly => Vec::new(),
            _ => ctx.take_entity_results(),
        };
        Ok(SearchResult::new(chunks, entities))
    }
}
