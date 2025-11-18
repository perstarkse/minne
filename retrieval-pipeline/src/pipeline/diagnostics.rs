use serde::Serialize;

/// Captures instrumentation for each hybrid retrieval stage when diagnostics are enabled.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PipelineDiagnostics {
    pub collect_candidates: Option<CollectCandidatesStats>,
    pub enrich_chunks_from_entities: Option<ChunkEnrichmentStats>,
    pub assemble: Option<AssembleStats>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CollectCandidatesStats {
    pub vector_entity_candidates: usize,
    pub vector_chunk_candidates: usize,
    pub fts_entity_candidates: usize,
    pub fts_chunk_candidates: usize,
    pub vector_chunk_scores: Vec<f32>,
    pub fts_chunk_scores: Vec<f32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChunkEnrichmentStats {
    pub filtered_entity_count: usize,
    pub fallback_min_results: usize,
    pub chunk_sources_considered: usize,
    pub chunk_candidates_before_enrichment: usize,
    pub chunk_candidates_after_enrichment: usize,
    pub top_chunk_scores: Vec<f32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AssembleStats {
    pub token_budget_start: usize,
    pub token_budget_spent: usize,
    pub token_budget_remaining: usize,
    pub budget_exhausted: bool,
    pub chunks_selected: usize,
    pub chunks_skipped_due_budget: usize,
    pub entity_count: usize,
    pub entity_traces: Vec<EntityAssemblyTrace>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EntityAssemblyTrace {
    pub entity_id: String,
    pub source_id: String,
    pub inspected_candidates: usize,
    pub selected_chunk_ids: Vec<String>,
    pub selected_chunk_scores: Vec<f32>,
    pub skipped_due_budget: usize,
}
