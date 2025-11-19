use std::collections::HashSet;

use chrono::{DateTime, Utc};
use retrieval_pipeline::{
    PipelineDiagnostics, PipelineStageTimings, RetrievedChunk, RetrievedEntity, StrategyOutput,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct EvaluationSummary {
    pub generated_at: DateTime<Utc>,
    pub k: usize,
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_label: Option<String>,
    pub total_cases: usize,
    pub correct: usize,
    pub precision: f64,
    pub correct_at_1: usize,
    pub correct_at_2: usize,
    pub correct_at_3: usize,
    pub precision_at_1: f64,
    pub precision_at_2: f64,
    pub precision_at_3: f64,
    pub duration_ms: u128,
    pub dataset_id: String,
    pub dataset_label: String,
    pub dataset_includes_unanswerable: bool,
    pub dataset_source: String,
    pub includes_impossible_cases: bool,
    pub require_verified_chunks: bool,
    pub filtered_questions: usize,
    pub retrieval_cases: usize,
    pub retrieval_correct: usize,
    pub retrieval_precision: f64,
    pub llm_cases: usize,
    pub llm_answered: usize,
    pub llm_precision: f64,
    pub slice_id: String,
    pub slice_seed: u64,
    pub slice_total_cases: usize,
    pub slice_window_offset: usize,
    pub slice_window_length: usize,
    pub slice_cases: usize,
    pub slice_positive_paragraphs: usize,
    pub slice_negative_paragraphs: usize,
    pub slice_total_paragraphs: usize,
    pub slice_negative_multiplier: f32,
    pub namespace_reused: bool,
    pub corpus_paragraphs: usize,
    pub ingestion_cache_path: String,
    pub ingestion_reused: bool,
    pub ingestion_embeddings_reused: bool,
    pub ingestion_fingerprint: String,
    pub positive_paragraphs_reused: usize,
    pub negative_paragraphs_reused: usize,
    pub latency_ms: LatencyStats,
    pub perf: PerformanceTimings,
    pub embedding_backend: String,
    pub embedding_model: Option<String>,
    pub embedding_dimension: usize,
    pub rerank_enabled: bool,
    pub rerank_pool_size: Option<usize>,
    pub rerank_keep_top: usize,
    pub concurrency: usize,
    pub detailed_report: bool,
    pub retrieval_strategy: String,
    pub chunk_vector_take: usize,
    pub chunk_fts_take: usize,
    pub chunk_token_budget: usize,
    pub chunk_avg_chars_per_token: usize,
    pub max_chunks_per_entity: usize,
    pub cases: Vec<CaseSummary>,
}

#[derive(Debug, Serialize)]
pub struct CaseSummary {
    pub question_id: String,
    pub question: String,
    pub paragraph_id: String,
    pub paragraph_title: String,
    pub expected_source: String,
    pub answers: Vec<String>,
    pub matched: bool,
    pub entity_match: bool,
    pub chunk_text_match: bool,
    pub chunk_id_match: bool,
    pub is_impossible: bool,
    pub has_verified_chunks: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_rank: Option<usize>,
    pub latency_ms: u128,
    pub retrieved: Vec<RetrievedSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub avg: f64,
    pub p50: u128,
    pub p95: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct StageLatencyBreakdown {
    pub embed: LatencyStats,
    pub collect_candidates: LatencyStats,
    pub graph_expansion: LatencyStats,
    pub chunk_attach: LatencyStats,
    pub rerank: LatencyStats,
    pub assemble: LatencyStats,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct EvaluationStageTimings {
    pub prepare_slice_ms: u128,
    pub prepare_db_ms: u128,
    pub prepare_corpus_ms: u128,
    pub prepare_namespace_ms: u128,
    pub run_queries_ms: u128,
    pub summarize_ms: u128,
    pub finalize_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct PerformanceTimings {
    pub openai_base_url: String,
    pub ingestion_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_seed_ms: Option<u128>,
    pub evaluation_stage_ms: EvaluationStageTimings,
    pub stage_latency: StageLatencyBreakdown,
}

#[derive(Debug, Serialize)]
pub struct RetrievedSummary {
    pub rank: usize,
    pub entity_id: String,
    pub source_id: String,
    pub entity_name: String,
    pub score: f32,
    pub matched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_text_match: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id_match: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct EvaluationCandidate {
    pub entity_id: String,
    pub source_id: String,
    pub entity_name: String,
    pub entity_description: Option<String>,
    pub entity_category: Option<String>,
    pub score: f32,
    pub chunks: Vec<RetrievedChunk>,
}

impl EvaluationCandidate {
    fn from_entity(entity: RetrievedEntity) -> Self {
        let entity_category = Some(format!("{:?}", entity.entity.entity_type));
        Self {
            entity_id: entity.entity.id.clone(),
            source_id: entity.entity.source_id.clone(),
            entity_name: entity.entity.name.clone(),
            entity_description: Some(entity.entity.description.clone()),
            entity_category,
            score: entity.score,
            chunks: entity.chunks,
        }
    }

    fn from_chunk(chunk: RetrievedChunk) -> Self {
        let snippet = chunk_snippet(&chunk.chunk.chunk);
        Self {
            entity_id: chunk.chunk.id.clone(),
            source_id: chunk.chunk.source_id.clone(),
            entity_name: chunk.chunk.source_id.clone(),
            entity_description: Some(snippet),
            entity_category: Some("Chunk".to_string()),
            score: chunk.score,
            chunks: vec![chunk],
        }
    }
}

pub fn adapt_strategy_output(output: StrategyOutput) -> Vec<EvaluationCandidate> {
    match output {
        StrategyOutput::Entities(entities) => entities
            .into_iter()
            .map(EvaluationCandidate::from_entity)
            .collect(),
        StrategyOutput::Chunks(chunks) => chunks
            .into_iter()
            .map(EvaluationCandidate::from_chunk)
            .collect(),
    }
}

#[derive(Debug, Serialize)]
pub struct CaseDiagnostics {
    pub question_id: String,
    pub question: String,
    pub paragraph_id: String,
    pub paragraph_title: String,
    pub expected_source: String,
    pub expected_chunk_ids: Vec<String>,
    pub answers: Vec<String>,
    pub entity_match: bool,
    pub chunk_text_match: bool,
    pub chunk_id_match: bool,
    pub failure_reasons: Vec<String>,
    pub missing_expected_chunk_ids: Vec<String>,
    pub attached_chunk_ids: Vec<String>,
    pub retrieved: Vec<EntityDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineDiagnostics>,
}

#[derive(Debug, Serialize)]
pub struct EntityDiagnostics {
    pub rank: usize,
    pub entity_id: String,
    pub source_id: String,
    pub name: String,
    pub score: f32,
    pub entity_match: bool,
    pub chunk_text_match: bool,
    pub chunk_id_match: bool,
    pub chunks: Vec<ChunkDiagnosticsEntry>,
}

#[derive(Debug, Serialize)]
pub struct ChunkDiagnosticsEntry {
    pub chunk_id: String,
    pub score: f32,
    pub contains_answer: bool,
    pub expected_chunk: bool,
    pub snippet: String,
}

pub fn text_contains_answer(text: &str, answers: &[String]) -> bool {
    if answers.is_empty() {
        return true;
    }
    let haystack = text.to_ascii_lowercase();
    answers.iter().any(|needle| haystack.contains(needle))
}

fn chunk_snippet(text: &str) -> String {
    const MAX_CHARS: usize = 160;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    let mut acc = String::with_capacity(MAX_CHARS + 3);
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= MAX_CHARS {
            break;
        }
        acc.push(ch);
    }
    acc.push_str("...");
    acc
}

pub fn compute_latency_stats(latencies: &[u128]) -> LatencyStats {
    if latencies.is_empty() {
        return LatencyStats {
            avg: 0.0,
            p50: 0,
            p95: 0,
        };
    }
    let mut sorted = latencies.to_vec();
    sorted.sort_unstable();
    let sum: u128 = sorted.iter().copied().sum();
    let avg = sum as f64 / (sorted.len() as f64);
    let p50 = percentile(&sorted, 0.50);
    let p95 = percentile(&sorted, 0.95);
    LatencyStats { avg, p50, p95 }
}

pub fn build_stage_latency_breakdown(samples: &[PipelineStageTimings]) -> StageLatencyBreakdown {
    fn collect_stage<F>(samples: &[PipelineStageTimings], selector: F) -> Vec<u128>
    where
        F: Fn(&PipelineStageTimings) -> u128,
    {
        samples.iter().map(selector).collect()
    }

    StageLatencyBreakdown {
        embed: compute_latency_stats(&collect_stage(samples, |entry| entry.embed_ms())),
        collect_candidates: compute_latency_stats(&collect_stage(samples, |entry| {
            entry.collect_candidates_ms()
        })),
        graph_expansion: compute_latency_stats(&collect_stage(samples, |entry| {
            entry.graph_expansion_ms()
        })),
        chunk_attach: compute_latency_stats(&collect_stage(samples, |entry| entry.chunk_attach_ms())),
        rerank: compute_latency_stats(&collect_stage(samples, |entry| entry.rerank_ms())),
        assemble: compute_latency_stats(&collect_stage(samples, |entry| entry.assemble_ms())),
    }
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let clamped = fraction.clamp(0.0, 1.0);
    let idx = (clamped * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn build_case_diagnostics(
    summary: &CaseSummary,
    expected_chunk_ids: &[String],
    answers_lower: &[String],
    candidates: &[EvaluationCandidate],
    pipeline_stats: Option<PipelineDiagnostics>,
) -> CaseDiagnostics {
    let expected_set: HashSet<&str> = expected_chunk_ids.iter().map(|id| id.as_str()).collect();
    let mut seen_chunks: HashSet<String> = HashSet::new();
    let mut attached_chunk_ids = Vec::new();
    let mut entity_diagnostics = Vec::new();

    for (idx, candidate) in candidates.iter().enumerate() {
        let mut chunk_entries = Vec::new();
        for chunk in &candidate.chunks {
            let contains_answer = text_contains_answer(&chunk.chunk.chunk, answers_lower);
            let expected_chunk = expected_set.contains(chunk.chunk.id.as_str());
            seen_chunks.insert(chunk.chunk.id.clone());
            attached_chunk_ids.push(chunk.chunk.id.clone());
            chunk_entries.push(ChunkDiagnosticsEntry {
                chunk_id: chunk.chunk.id.clone(),
                score: chunk.score,
                contains_answer,
                expected_chunk,
                snippet: chunk_snippet(&chunk.chunk.chunk),
            });
        }
        entity_diagnostics.push(EntityDiagnostics {
            rank: idx + 1,
            entity_id: candidate.entity_id.clone(),
            source_id: candidate.source_id.clone(),
            name: candidate.entity_name.clone(),
            score: candidate.score,
            entity_match: candidate.source_id == summary.expected_source,
            chunk_text_match: chunk_entries.iter().any(|entry| entry.contains_answer),
            chunk_id_match: chunk_entries.iter().any(|entry| entry.expected_chunk),
            chunks: chunk_entries,
        });
    }

    let missing_expected_chunk_ids = expected_chunk_ids
        .iter()
        .filter(|id| !seen_chunks.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    let mut failure_reasons = Vec::new();
    if !summary.entity_match {
        failure_reasons.push("entity_miss".to_string());
    }
    if !summary.chunk_text_match {
        failure_reasons.push("chunk_text_missing".to_string());
    }
    if !summary.chunk_id_match {
        failure_reasons.push("chunk_id_missing".to_string());
    }
    if !missing_expected_chunk_ids.is_empty() {
        failure_reasons.push("expected_chunk_absent".to_string());
    }

    CaseDiagnostics {
        question_id: summary.question_id.clone(),
        question: summary.question.clone(),
        paragraph_id: summary.paragraph_id.clone(),
        paragraph_title: summary.paragraph_title.clone(),
        expected_source: summary.expected_source.clone(),
        expected_chunk_ids: expected_chunk_ids.to_vec(),
        answers: summary.answers.clone(),
        entity_match: summary.entity_match,
        chunk_text_match: summary.chunk_text_match,
        chunk_id_match: summary.chunk_id_match,
        failure_reasons,
        missing_expected_chunk_ids,
        attached_chunk_ids,
        retrieved: entity_diagnostics,
        pipeline: pipeline_stats,
    }
}
