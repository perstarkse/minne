//! Case generation from corpus manifests.

use std::collections::HashMap;

use crate::corpus;

/// A test case for retrieval evaluation derived from a manifest question.
pub(crate) struct SeededCase {
    pub question_id: String,
    pub question: String,
    pub expected_source: String,
    pub answers: Vec<String>,
    pub paragraph_id: String,
    pub paragraph_title: String,
    pub expected_chunk_ids: Vec<String>,
    pub is_impossible: bool,
    pub has_verified_chunks: bool,
}

/// Convert a corpus manifest into seeded evaluation cases.
pub(crate) fn cases_from_manifest(manifest: &corpus::CorpusManifest) -> Vec<SeededCase> {
    let mut title_map = HashMap::new();
    for paragraph in &manifest.paragraphs {
        title_map.insert(paragraph.paragraph_id.as_str(), paragraph.title.clone());
    }

    let include_impossible = manifest.metadata.include_unanswerable;
    let require_verified_chunks = manifest.metadata.require_verified_chunks;

    manifest
        .questions
        .iter()
        .filter(|question| {
            should_include_question(question, include_impossible, require_verified_chunks)
        })
        .map(|question| {
            let title = title_map
                .get(question.paragraph_id.as_str())
                .cloned()
                .unwrap_or_else(|| "Untitled".to_string());
            SeededCase {
                question_id: question.question_id.clone(),
                question: question.question_text.clone(),
                expected_source: question.text_content_id.clone(),
                answers: question.answers.clone(),
                paragraph_id: question.paragraph_id.clone(),
                paragraph_title: title,
                expected_chunk_ids: question.matching_chunk_ids.clone(),
                is_impossible: question.is_impossible,
                has_verified_chunks: !question.matching_chunk_ids.is_empty(),
            }
        })
        .collect()
}

fn should_include_question(
    question: &corpus::CorpusQuestion,
    include_impossible: bool,
    require_verified_chunks: bool,
) -> bool {
    if !include_impossible && question.is_impossible {
        return false;
    }
    if require_verified_chunks && question.matching_chunk_ids.is_empty() {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::store::{CorpusParagraph, EmbeddedKnowledgeEntity, EmbeddedTextChunk};
    use crate::corpus::{CorpusManifest, CorpusMetadata, CorpusQuestion, MANIFEST_VERSION};
    use chrono::Utc;
    use common::storage::types::text_content::TextContent;

    fn sample_manifest() -> CorpusManifest {
        let paragraphs = vec![
            CorpusParagraph {
                paragraph_id: "p1".to_string(),
                title: "Alpha".to_string(),
                text_content: TextContent::new(
                    "alpha context".to_string(),
                    None,
                    "test".to_string(),
                    None,
                    None,
                    "user".to_string(),
                ),
                entities: Vec::<EmbeddedKnowledgeEntity>::new(),
                relationships: Vec::new(),
                chunks: Vec::<EmbeddedTextChunk>::new(),
            },
            CorpusParagraph {
                paragraph_id: "p2".to_string(),
                title: "Beta".to_string(),
                text_content: TextContent::new(
                    "beta context".to_string(),
                    None,
                    "test".to_string(),
                    None,
                    None,
                    "user".to_string(),
                ),
                entities: Vec::<EmbeddedKnowledgeEntity>::new(),
                relationships: Vec::new(),
                chunks: Vec::<EmbeddedTextChunk>::new(),
            },
        ];
        let questions = vec![
            CorpusQuestion {
                question_id: "q1".to_string(),
                paragraph_id: "p1".to_string(),
                text_content_id: "tc-alpha".to_string(),
                question_text: "What is Alpha?".to_string(),
                answers: vec!["Alpha".to_string()],
                is_impossible: false,
                matching_chunk_ids: vec!["chunk-alpha".to_string()],
            },
            CorpusQuestion {
                question_id: "q2".to_string(),
                paragraph_id: "p1".to_string(),
                text_content_id: "tc-alpha".to_string(),
                question_text: "Unanswerable?".to_string(),
                answers: Vec::new(),
                is_impossible: true,
                matching_chunk_ids: Vec::new(),
            },
            CorpusQuestion {
                question_id: "q3".to_string(),
                paragraph_id: "p2".to_string(),
                text_content_id: "tc-beta".to_string(),
                question_text: "Where is Beta?".to_string(),
                answers: vec!["Beta".to_string()],
                is_impossible: false,
                matching_chunk_ids: Vec::new(),
            },
        ];
        CorpusManifest {
            version: MANIFEST_VERSION,
            metadata: CorpusMetadata {
                dataset_id: "ds".to_string(),
                dataset_label: "Dataset".to_string(),
                slice_id: "slice".to_string(),
                include_unanswerable: true,
                require_verified_chunks: true,
                ingestion_fingerprint: "fp".to_string(),
                embedding_backend: "test".to_string(),
                embedding_model: None,
                embedding_dimension: 3,
                converted_checksum: "chk".to_string(),
                generated_at: Utc::now(),
                paragraph_count: paragraphs.len(),
                question_count: questions.len(),
                chunk_min_tokens: 1,
                chunk_max_tokens: 10,
                chunk_only: false,
            },
            paragraphs,
            questions,
        }
    }

    #[test]
    fn cases_respect_mode_filters() {
        let mut manifest = sample_manifest();
        manifest.metadata.include_unanswerable = false;
        manifest.metadata.require_verified_chunks = true;

        let strict_cases = cases_from_manifest(&manifest);
        assert_eq!(strict_cases.len(), 1);
        assert_eq!(strict_cases[0].question_id, "q1");
        assert_eq!(strict_cases[0].paragraph_title, "Alpha");

        let mut llm_manifest = manifest.clone();
        llm_manifest.metadata.include_unanswerable = true;
        llm_manifest.metadata.require_verified_chunks = false;

        let llm_cases = cases_from_manifest(&llm_manifest);
        let ids: Vec<_> = llm_cases
            .iter()
            .map(|case| case.question_id.as_str())
            .collect();
        assert_eq!(ids, vec!["q1", "q2", "q3"]);
    }
}
