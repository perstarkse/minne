use serde::{Deserialize, Serialize};

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
    pub tuning: RetrievalTuning,
}

impl RetrievalConfig {
    pub fn new(tuning: RetrievalTuning) -> Self {
        Self { tuning }
    }
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            tuning: RetrievalTuning::default(),
        }
    }
}
