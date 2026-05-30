use serde::Serialize;

/// Captures instrumentation for the retrieval stages when diagnostics are enabled.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Diagnostics {
    pub search: Option<SearchStats>,
    pub assemble: Option<AssembleStats>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchStats {
    pub vector_chunk_candidates: usize,
    pub fts_chunk_candidates: usize,
    pub vector_chunk_scores: Vec<f32>,
    pub fts_chunk_scores: Vec<f32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AssembleStats {
    pub chunks_selected: usize,
}
