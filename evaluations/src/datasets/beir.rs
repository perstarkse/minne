use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tracing::warn;

use super::{ConvertedParagraph, ConvertedQuestion, DatasetKind};

const ANSWER_SNIPPET_CHARS: usize = 240;

#[derive(Debug, Deserialize)]
struct BeirCorpusRow {
    #[serde(rename = "_id")]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BeirQueryRow {
    #[serde(rename = "_id")]
    id: String,
    text: String,
}

#[derive(Debug, Clone)]
struct BeirParagraph {
    title: String,
    context: String,
}

#[derive(Debug, Clone)]
struct BeirQuery {
    text: String,
}

#[derive(Debug, Clone)]
struct QrelEntry {
    doc_id: String,
    score: i32,
}

/// Convert only documents that appear in qrels (the BEIR evaluation closed world).
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
pub fn convert_beir(raw_dir: &Path, dataset: DatasetKind) -> Result<Vec<ConvertedParagraph>> {
    convert_beir_documents(raw_dir, dataset, None)
}

/// Convert a subset of qrels-world documents. `doc_ids` use corpus ids (unprefixed).
#[allow(
    clippy::too_many_lines,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
pub fn convert_beir_documents(
    raw_dir: &Path,
    dataset: DatasetKind,
    doc_ids: Option<&HashSet<String>>,
) -> Result<Vec<ConvertedParagraph>> {
    let corpus_path = raw_dir.join("corpus.jsonl");
    let queries_path = raw_dir.join("queries.jsonl");
    let qrels_path = resolve_qrels_path(raw_dir)?;

    let queries = load_queries(&queries_path)?;
    let qrels = load_qrels(&qrels_path)?;

    let mut qrels_doc_ids = HashSet::new();
    for entries in qrels.values() {
        for entry in entries {
            qrels_doc_ids.insert(entry.doc_id.clone());
        }
    }

    let target_doc_ids: HashSet<String> = match doc_ids {
        Some(ids) => ids
            .iter()
            .filter(|id| qrels_doc_ids.contains(*id))
            .cloned()
            .collect(),
        None => qrels_doc_ids.clone(),
    };

    if target_doc_ids.is_empty() {
        return Err(anyhow!(
            "no qrels documents to convert for {} at {}",
            dataset.id(),
            raw_dir.display()
        ));
    }

    let corpus = load_corpus_filtered(&corpus_path, &target_doc_ids)?;

    let mut doc_ids_sorted: Vec<String> = target_doc_ids.into_iter().collect();
    doc_ids_sorted.sort();

    let mut paragraphs = Vec::with_capacity(doc_ids_sorted.len());
    let mut paragraph_index = HashMap::new();

    for doc_id in &doc_ids_sorted {
        let Some(entry) = corpus.get(doc_id) else {
            warn!(
                doc_id = %doc_id,
                dataset = %dataset.id(),
                "Skipping qrels document missing from corpus"
            );
            continue;
        };
        let paragraph_id = format!("{}-{doc_id}", dataset.source_prefix());
        let paragraph = ConvertedParagraph {
            id: paragraph_id.clone(),
            title: entry.title.clone(),
            context: entry.context.clone(),
            questions: Vec::new(),
        };
        paragraph_index.insert(doc_id.clone(), paragraphs.len());
        paragraphs.push(paragraph);
    }

    let mut missing_queries = 0usize;
    let mut missing_docs = 0usize;
    let mut skipped_answers = 0usize;

    for (query_id, entries) in qrels {
        let Some(query) = queries.get(&query_id) else {
            missing_queries += 1;
            warn!(query_id = %query_id, "Skipping qrels entry for missing query");
            continue;
        };

        let Some(best) = select_best_doc(&entries) else {
            continue;
        };

        if let Some(filter) = doc_ids {
            if !filter.contains(&best.doc_id) {
                continue;
            }
        }

        let Some(&paragraph_slot) = paragraph_index.get(&best.doc_id) else {
            missing_docs += 1;
            warn!(
                query_id = %query_id,
                doc_id = %best.doc_id,
                "Skipping qrels entry referencing missing corpus document"
            );
            continue;
        };

        let Some(snippet) = answer_snippet(&paragraphs[paragraph_slot].context) else {
            skipped_answers += 1;
            warn!(
                query_id = %query_id,
                doc_id = %best.doc_id,
                "Skipping query because no non-empty answer snippet could be derived"
            );
            continue;
        };

        let question_id = format!("{}-{query_id}", dataset.source_prefix());
        paragraphs[paragraph_slot]
            .questions
            .push(ConvertedQuestion {
                id: question_id,
                question: query.text.clone(),
                answers: vec![snippet],
                is_impossible: false,
            });
    }

    if missing_queries + missing_docs + skipped_answers > 0 {
        warn!(
            missing_queries,
            missing_docs,
            skipped_answers,
            dataset = %dataset.id(),
            "Skipped some BEIR qrels entries during conversion"
        );
    }

    Ok(paragraphs)
}

pub fn corpus_doc_id(paragraph_id: &str, dataset: DatasetKind) -> Option<String> {
    let prefix = format!("{}-", dataset.source_prefix());
    paragraph_id
        .strip_prefix(&prefix)
        .map(str::to_string)
}

fn resolve_qrels_path(raw_dir: &Path) -> Result<PathBuf> {
    let qrels_dir = raw_dir.join("qrels");
    let candidates = ["test.tsv", "dev.tsv", "train.tsv"];

    for name in candidates {
        let candidate = qrels_dir.join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(anyhow!(
        "No qrels file found under {}; expected one of {:?}",
        qrels_dir.display(),
        candidates
    ))
}

#[allow(clippy::arithmetic_side_effects)]
fn load_corpus_filtered(
    path: &Path,
    doc_ids: &HashSet<String>,
) -> Result<BTreeMap<String, BeirParagraph>> {
    let file =
        File::open(path).with_context(|| format!("opening BEIR corpus at {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut corpus = BTreeMap::new();

    for (idx, line) in reader.lines().enumerate() {
        let raw = line
            .with_context(|| format!("reading corpus line {} from {}", idx + 1, path.display()))?;
        if raw.trim().is_empty() {
            continue;
        }
        let corpus_row: BeirCorpusRow = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing corpus JSON on line {} from {}",
                idx + 1,
                path.display()
            )
        })?;
        if !doc_ids.contains(&corpus_row.id) {
            continue;
        }
        let title = corpus_row.title.unwrap_or_else(|| corpus_row.id.clone());
        let text = corpus_row.text.unwrap_or_default();
        let context = build_context(&title, &text);

        if context.is_empty() {
            warn!(doc_id = %corpus_row.id, "Skipping empty corpus document");
            continue;
        }

        corpus.insert(corpus_row.id, BeirParagraph { title, context });
    }

    Ok(corpus)
}

#[allow(clippy::arithmetic_side_effects)]
fn load_queries(path: &Path) -> Result<BTreeMap<String, BeirQuery>> {
    let file = File::open(path)
        .with_context(|| format!("opening BEIR queries file at {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut queries = BTreeMap::new();

    for (idx, line) in reader.lines().enumerate() {
        let raw = line
            .with_context(|| format!("reading query line {} from {}", idx + 1, path.display()))?;
        if raw.trim().is_empty() {
            continue;
        }
        let query_row: BeirQueryRow = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing query JSON on line {} from {}",
                idx + 1,
                path.display()
            )
        })?;
        queries.insert(
            query_row.id,
            BeirQuery {
                text: query_row.text.trim().to_string(),
            },
        );
    }

    Ok(queries)
}

#[allow(clippy::arithmetic_side_effects)]
fn load_qrels(path: &Path) -> Result<BTreeMap<String, Vec<QrelEntry>>> {
    let file =
        File::open(path).with_context(|| format!("opening BEIR qrels at {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut qrels: BTreeMap<String, Vec<QrelEntry>> = BTreeMap::new();

    for (idx, line) in reader.lines().enumerate() {
        let raw = line
            .with_context(|| format!("reading qrels line {} from {}", idx + 1, path.display()))?;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("query-id") {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let query_id = parts
            .next()
            .ok_or_else(|| anyhow!("missing query id on line {}", idx + 1))?;
        let doc_id = parts
            .next()
            .ok_or_else(|| anyhow!("missing document id on line {}", idx + 1))?;
        let score_raw = parts
            .next()
            .ok_or_else(|| anyhow!("missing score on line {}", idx + 1))?;
        let score: i32 = score_raw.parse().with_context(|| {
            format!(
                "parsing qrels score '{}' on line {} from {}",
                score_raw,
                idx + 1,
                path.display()
            )
        })?;

        qrels
            .entry(query_id.to_string())
            .or_default()
            .push(QrelEntry {
                doc_id: doc_id.to_string(),
                score,
            });
    }

    Ok(qrels)
}

fn select_best_doc(entries: &[QrelEntry]) -> Option<&QrelEntry> {
    entries
        .iter()
        .max_by(|a, b| a.score.cmp(&b.score).then_with(|| b.doc_id.cmp(&a.doc_id)))
}

fn answer_snippet(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let snippet: String = trimmed.chars().take(ANSWER_SNIPPET_CHARS).collect();
    let snippet = snippet.trim();
    if snippet.is_empty() {
        None
    } else {
        Some(snippet.to_string())
    }
}

fn build_context(title: &str, text: &str) -> String {
    let title = title.trim();
    let text = text.trim();

    match (title.is_empty(), text.is_empty()) {
        (true, true) => String::new(),
        (true, false) => text.to_string(),
        (false, true) => title.to_string(),
        (false, false) => format!("{title}\n\n{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[allow(clippy::unwrap_used)]
    fn write_fixture(dir: &tempfile::TempDir) {
        let corpus = r#"
{"_id":"d1","title":"Doc 1","text":"Doc one has some text for testing."}
{"_id":"d2","title":"Doc 2","text":"Second document content."}
"#;
        let queries = r#"
{"_id":"q1","text":"What is in doc one?"}
"#;
        let qrels = "query-id\tcorpus-id\tscore\nq1\td1\t2\n";

        fs::write(dir.path().join("corpus.jsonl"), corpus.trim()).unwrap();
        fs::write(dir.path().join("queries.jsonl"), queries.trim()).unwrap();
        fs::create_dir_all(dir.path().join("qrels")).unwrap();
        fs::write(dir.path().join("qrels/test.tsv"), qrels).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    fn converts_qrels_world_only() {
        let dir = tempdir().unwrap();
        write_fixture(&dir);

        let paragraphs = convert_beir(dir.path(), DatasetKind::Fever).unwrap();

        assert_eq!(paragraphs.len(), 1);
        let doc_one = &paragraphs[0];
        assert_eq!(doc_one.id, "fever-d1");
        assert_eq!(doc_one.questions.len(), 1);
        assert_eq!(doc_one.questions[0].id, "fever-q1");
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    fn converts_filtered_doc_ids() {
        let dir = tempdir().unwrap();
        write_fixture(&dir);

        let mut ids = HashSet::new();
        ids.insert("d1".to_string());
        let paragraphs =
            convert_beir_documents(dir.path(), DatasetKind::Fever, Some(&ids)).unwrap();
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].id, "fever-d1");
    }
}
