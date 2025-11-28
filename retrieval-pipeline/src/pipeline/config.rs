use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStrategy {
    Initial,
    Revised,
    RelationshipSuggestion,
    Ingestion,
}

impl Default for RetrievalStrategy {
    fn default() -> Self {
        Self::Initial
    }
}

impl std::str::FromStr for RetrievalStrategy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "initial" => Ok(Self::Initial),
            "revised" => Ok(Self::Revised),
            "relationship_suggestion" => Ok(Self::RelationshipSuggestion),
            "ingestion" => Ok(Self::Ingestion),
            other => Err(format!("unknown retrieval strategy '{other}'")),
        }
    }
}

impl fmt::Display for RetrievalStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            RetrievalStrategy::Initial => "initial",
            RetrievalStrategy::Revised => "revised",
            RetrievalStrategy::RelationshipSuggestion => "relationship_suggestion",
            RetrievalStrategy::Ingestion => "ingestion",
        };
        f.write_str(label)
    }
}

/// Tunable parameters that govern each retrieval stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalTuning {
    pub entity_vector_take: usize,
    pub chunk_vector_take: usize,
    pub entity_fts_take: usize,
    pub chunk_fts_take: usize,
    pub score_threshold: f32,
    pub fallback_min_results: usize,
    pub token_budget_estimate: usize,
    pub avg_chars_per_token: usize,
    pub max_chunks_per_entity: usize,
    pub lexical_match_weight: f32,
    pub graph_traversal_seed_limit: usize,
    pub graph_neighbor_limit: usize,
    pub graph_score_decay: f32,
    pub graph_seed_min_score: f32,
    pub graph_vector_inheritance: f32,
    pub rerank_blend_weight: f32,
    pub rerank_scores_only: bool,
    pub rerank_keep_top: usize,
}

impl Default for RetrievalTuning {
    fn default() -> Self {
        Self {
            entity_vector_take: 15,
            chunk_vector_take: 20,
            entity_fts_take: 10,
            chunk_fts_take: 20,
            score_threshold: 0.35,
            fallback_min_results: 10,
            token_budget_estimate: 10000,
            avg_chars_per_token: 4,
            max_chunks_per_entity: 4,
            lexical_match_weight: 0.15,
            graph_traversal_seed_limit: 5,
            graph_neighbor_limit: 6,
            graph_score_decay: 0.75,
            graph_seed_min_score: 0.4,
            graph_vector_inheritance: 0.6,
            rerank_blend_weight: 0.65,
            rerank_scores_only: false,
            rerank_keep_top: 8,
        }
    }
}

/// Wrapper containing tuning plus future flags for per-request overrides.
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    pub strategy: RetrievalStrategy,
    pub tuning: RetrievalTuning,
}

impl RetrievalConfig {
    pub fn new(tuning: RetrievalTuning) -> Self {
        Self {
            strategy: RetrievalStrategy::Initial,
            tuning,
        }
    }

    pub fn with_strategy(strategy: RetrievalStrategy) -> Self {
        Self {
            strategy,
            tuning: RetrievalTuning::default(),
        }
    }

    pub fn with_tuning(strategy: RetrievalStrategy, tuning: RetrievalTuning) -> Self {
        Self { strategy, tuning }
    }

    /// Create config for chat retrieval with strategy selection support
    pub fn for_chat(strategy: RetrievalStrategy) -> Self {
        Self::with_strategy(strategy)
    }

    /// Create config for relationship suggestion (entity-only retrieval)
    pub fn for_relationship_suggestion() -> Self {
        Self::with_strategy(RetrievalStrategy::RelationshipSuggestion)
    }

    /// Create config for ingestion pipeline (entity-only retrieval)
    pub fn for_ingestion() -> Self {
        Self::with_strategy(RetrievalStrategy::Ingestion)
    }
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            strategy: RetrievalStrategy::default(),
            tuning: RetrievalTuning::default(),
        }
    }
}
