use serde::{Deserialize, Serialize};
use std::fmt;

use crate::scoring::FusionWeights;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStrategy {
    /// Primary hybrid chunk retrieval for search/chat (formerly Revised)
    Default,
    /// Entity retrieval for suggesting relationships when creating manual entities
    RelationshipSuggestion,
    /// Entity retrieval for context during content ingestion
    Ingestion,
    /// Unified search returning both chunks and entities
    Search,
}

/// Configures which result types to include in Search strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchTarget {
    /// Return only text chunks
    ChunksOnly,
    /// Return only knowledge entities
    EntitiesOnly,
    /// Return both chunks and entities (default)
    #[default]
    Both,
}

impl Default for RetrievalStrategy {
    fn default() -> Self {
        Self::Default
    }
}

impl std::str::FromStr for RetrievalStrategy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "default" => Ok(Self::Default),
            // Backward compatibility: treat "initial" and "revised" as "default"
            "initial" | "revised" => {
                tracing::warn!(
                    "Retrieval strategy '{}' is deprecated. Use 'default' instead. \
                     The 'initial' strategy has been removed in favor of the simpler hybrid chunk retrieval.",
                    value
                );
                Ok(Self::Default)
            }
            "relationship_suggestion" => Ok(Self::RelationshipSuggestion),
            "ingestion" => Ok(Self::Ingestion),
            "search" => Ok(Self::Search),
            other => Err(format!("unknown retrieval strategy '{other}'")),
        }
    }
}

impl fmt::Display for RetrievalStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            RetrievalStrategy::Default => "default",
            RetrievalStrategy::RelationshipSuggestion => "relationship_suggestion",
            RetrievalStrategy::Ingestion => "ingestion",
            RetrievalStrategy::Search => "search",
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
    pub chunk_result_cap: usize,
    /// Optional fusion weights for hybrid search. If None, uses default weights.
    pub fusion_weights: Option<FusionWeights>,
    /// Normalize vector similarity scores before fusion (default: true)
    pub normalize_vector_scores: bool,
    /// Normalize FTS (BM25) scores before fusion (default: true)
    pub normalize_fts_scores: bool,
    /// Reciprocal rank fusion k value for chunk merging in Revised strategy.
    #[serde(default = "default_chunk_rrf_k")]
    pub chunk_rrf_k: f32,
    /// Weight applied to vector ranks in RRF.
    #[serde(default = "default_chunk_rrf_vector_weight")]
    pub chunk_rrf_vector_weight: f32,
    /// Weight applied to chunk FTS ranks in RRF.
    #[serde(default = "default_chunk_rrf_fts_weight")]
    pub chunk_rrf_fts_weight: f32,
    /// Whether to include vector rankings in RRF.
    #[serde(default = "default_chunk_rrf_use_vector")]
    pub chunk_rrf_use_vector: bool,
    /// Whether to include chunk FTS rankings in RRF.
    #[serde(default = "default_chunk_rrf_use_fts")]
    pub chunk_rrf_use_fts: bool,
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
            chunk_result_cap: 5,
            fusion_weights: None,
            // Vector scores (cosine similarity) are already in [0,1] range
            // Normalization only helps when there's significant variation
            normalize_vector_scores: false,
            // FTS scores (BM25) are unbounded, normalization helps more
            normalize_fts_scores: true,
            chunk_rrf_k: default_chunk_rrf_k(),
            chunk_rrf_vector_weight: default_chunk_rrf_vector_weight(),
            chunk_rrf_fts_weight: default_chunk_rrf_fts_weight(),
            chunk_rrf_use_vector: default_chunk_rrf_use_vector(),
            chunk_rrf_use_fts: default_chunk_rrf_use_fts(),
        }
    }
}

/// Wrapper containing tuning plus future flags for per-request overrides.
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    pub strategy: RetrievalStrategy,
    pub tuning: RetrievalTuning,
    /// Target for Search strategy (chunks, entities, or both)
    pub search_target: SearchTarget,
}

impl RetrievalConfig {
    pub fn new(tuning: RetrievalTuning) -> Self {
        Self {
            strategy: RetrievalStrategy::Default,
            tuning,
            search_target: SearchTarget::default(),
        }
    }

    pub fn with_strategy(strategy: RetrievalStrategy) -> Self {
        Self {
            strategy,
            tuning: RetrievalTuning::default(),
            search_target: SearchTarget::default(),
        }
    }

    pub fn with_tuning(strategy: RetrievalStrategy, tuning: RetrievalTuning) -> Self {
        Self {
            strategy,
            tuning,
            search_target: SearchTarget::default(),
        }
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

    /// Create config for unified search (chunks and/or entities)
    pub fn for_search(target: SearchTarget) -> Self {
        Self {
            strategy: RetrievalStrategy::Search,
            tuning: RetrievalTuning::default(),
            search_target: target,
        }
    }
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            strategy: RetrievalStrategy::default(),
            tuning: RetrievalTuning::default(),
            search_target: SearchTarget::default(),
        }
    }
}

const fn default_chunk_rrf_k() -> f32 {
    60.0
}

const fn default_chunk_rrf_vector_weight() -> f32 {
    1.0
}

const fn default_chunk_rrf_fts_weight() -> f32 {
    1.0
}

const fn default_chunk_rrf_use_vector() -> bool {
    true
}

const fn default_chunk_rrf_use_fts() -> bool {
    true
}
