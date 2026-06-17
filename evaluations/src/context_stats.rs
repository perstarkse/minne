use serde::{Deserialize, Serialize};

use common::storage::types::StoredObject;

use crate::types::EvaluationCandidate;

const TOKENIZER_LABEL: &str = "estimated (~chars/4; ingestion uses bert-base-cased)";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetrievedContextStats {
    pub chunk_count: usize,
    pub char_count: usize,
    pub token_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalContextStats {
    pub tokenizer: String,
    pub queries: usize,
    pub total_chunks: usize,
    pub total_chars: usize,
    pub total_tokens: usize,
    pub avg_chunks_per_query: f64,
    pub avg_chars_per_query: f64,
    pub avg_tokens_per_query: f64,
    pub p50_tokens_per_query: usize,
    pub p95_tokens_per_query: usize,
    pub max_tokens_per_query: usize,
}

pub fn stats_for_candidates(candidates: &[EvaluationCandidate]) -> RetrievedContextStats {
    let mut seen_chunk_ids = std::collections::HashSet::new();
    let mut stats = RetrievedContextStats::default();

    for candidate in candidates {
        for chunk in &candidate.chunks {
            let chunk_id = chunk.chunk.id().to_string();
            if !seen_chunk_ids.insert(chunk_id) {
                continue;
            }
            let text = chunk.chunk.chunk.as_str();
            stats.chunk_count += 1;
            stats.char_count += text.chars().count();
            stats.token_count += estimate_ingestion_tokens(text);
        }
    }

    stats
}

pub fn aggregate_context_stats(per_query: &[RetrievedContextStats]) -> RetrievalContextStats {
    let queries = per_query.len();
    if queries == 0 {
        return RetrievalContextStats {
            tokenizer: TOKENIZER_LABEL.to_string(),
            queries: 0,
            total_chunks: 0,
            total_chars: 0,
            total_tokens: 0,
            avg_chunks_per_query: 0.0,
            avg_chars_per_query: 0.0,
            avg_tokens_per_query: 0.0,
            p50_tokens_per_query: 0,
            p95_tokens_per_query: 0,
            max_tokens_per_query: 0,
        };
    }

    let total_chunks: usize = per_query.iter().map(|stats| stats.chunk_count).sum();
    let total_chars: usize = per_query.iter().map(|stats| stats.char_count).sum();
    let total_tokens: usize = per_query.iter().map(|stats| stats.token_count).sum();
    let mut tokens_per_query: Vec<usize> = per_query.iter().map(|stats| stats.token_count).collect();
    tokens_per_query.sort_unstable();
    let max_tokens_per_query = *tokens_per_query.last().unwrap_or(&0);

    RetrievalContextStats {
        tokenizer: TOKENIZER_LABEL.to_string(),
        queries,
        total_chunks,
        total_chars,
        total_tokens,
        avg_chunks_per_query: total_chunks as f64 / queries as f64,
        avg_chars_per_query: total_chars as f64 / queries as f64,
        avg_tokens_per_query: total_tokens as f64 / queries as f64,
        p50_tokens_per_query: percentile_usize(&tokens_per_query, 0.50),
        p95_tokens_per_query: percentile_usize(&tokens_per_query, 0.95),
        max_tokens_per_query,
    }
}

fn estimate_ingestion_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        return 0;
    }
    chars.div_ceil(4)
}

#[allow(clippy::cast_precision_loss, clippy::indexing_slicing, clippy::arithmetic_side_effects)]
fn percentile_usize(sorted: &[usize], fraction: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let clamped = fraction.clamp(0.0, 1.0);
    let index = ((sorted.len() - 1) as f64 * clamped).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use common::storage::types::text_chunk::TextChunk;
    use retrieval_pipeline::RetrievedChunk;

    #[test]
    fn deduplicates_chunks_when_counting_context() {
        let shared = Arc::new(TextChunk::new(
            "src".into(),
            "hello world".into(),
            "user".into(),
        ));
        let candidates = vec![
            EvaluationCandidate {
                entity_id: "a".into(),
                source_id: "src".into(),
                entity_name: "A".into(),
                entity_description: None,
                entity_category: None,
                score: 1.0,
                chunks: vec![RetrievedChunk {
                    chunk: Arc::clone(&shared),
                    score: 1.0,
                }],
            },
            EvaluationCandidate {
                entity_id: "b".into(),
                source_id: "src".into(),
                entity_name: "B".into(),
                entity_description: None,
                entity_category: None,
                score: 0.9,
                chunks: vec![RetrievedChunk {
                    chunk: shared,
                    score: 0.9,
                }],
            },
        ];
        let stats = stats_for_candidates(&candidates);
        assert_eq!(stats.chunk_count, 1);
        assert_eq!(stats.char_count, "hello world".chars().count());
        assert_eq!(stats.token_count, 3);
    }

    #[test]
    fn aggregates_per_query_token_totals() {
        let per_query = vec![
            RetrievedContextStats {
                chunk_count: 2,
                char_count: 100,
                token_count: 40,
            },
            RetrievedContextStats {
                chunk_count: 5,
                char_count: 250,
                token_count: 100,
            },
        ];
        let aggregate = aggregate_context_stats(&per_query);
        assert_eq!(aggregate.queries, 2);
        assert_eq!(aggregate.total_chunks, 7);
        assert_eq!(aggregate.total_tokens, 140);
        assert_eq!(aggregate.max_tokens_per_query, 100);
        assert!((aggregate.avg_tokens_per_query - 70.0).abs() < f64::EPSILON);
    }
}
