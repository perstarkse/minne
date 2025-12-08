use std::{
    collections::{BTreeMap, HashMap},
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

pub fn convert_beir(raw_dir: &Path, dataset: DatasetKind) -> Result<Vec<ConvertedParagraph>> {
    let corpus_path = raw_dir.join("corpus.jsonl");
    let queries_path = raw_dir.join("queries.jsonl");
    let qrels_path = resolve_qrels_path(raw_dir)?;

    let corpus = load_corpus(&corpus_path)?;
    let queries = load_queries(&queries_path)?;
    let qrels = load_qrels(&qrels_path)?;

    let mut paragraphs = Vec::with_capacity(corpus.len());
    let mut paragraph_index = HashMap::new();

    for (doc_id, entry) in corpus.iter() {
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
        let query = match queries.get(&query_id) {
            Some(query) => query,
            None => {
                missing_queries += 1;
                warn!(query_id = %query_id, "Skipping qrels entry for missing query");
                continue;
            }
        };

        let best = match select_best_doc(&entries) {
            Some(entry) => entry,
            None => continue,
        };

        let paragraph_slot = match paragraph_index.get(&best.doc_id) {
            Some(slot) => *slot,
            None => {
                missing_docs += 1;
                warn!(
                    query_id = %query_id,
                    doc_id = %best.doc_id,
                    "Skipping qrels entry referencing missing corpus document"
                );
                continue;
            }
        };

        let answer = answer_snippet(&paragraphs[paragraph_slot].context);
        let answers = match answer {
            Some(snippet) => vec![snippet],
            None => {
                skipped_answers += 1;
                warn!(
                    query_id = %query_id,
                    doc_id = %best.doc_id,
                    "Skipping query because no non-empty answer snippet could be derived"
                );
                continue;
            }
        };

        let question_id = format!("{}-{query_id}", dataset.source_prefix());
        paragraphs[paragraph_slot]
            .questions
            .push(ConvertedQuestion {
                id: question_id,
                question: query.text.clone(),
                answers,
                is_impossible: false,
            });
    }

    if missing_queries + missing_docs + skipped_answers > 0 {
        warn!(
            missing_queries,
            missing_docs, skipped_answers, "Skipped some BEIR qrels entries during conversion"
        );
    }

    Ok(paragraphs)
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

fn load_corpus(path: &Path) -> Result<BTreeMap<String, BeirParagraph>> {
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
        let row: BeirCorpusRow = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing corpus JSON on line {} from {}",
                idx + 1,
                path.display()
            )
        })?;
        let title = row.title.unwrap_or_else(|| row.id.clone());
        let text = row.text.unwrap_or_default();
        let context = build_context(&title, &text);

        if context.is_empty() {
            warn!(doc_id = %row.id, "Skipping empty corpus document");
            continue;
        }

        corpus.insert(row.id, BeirParagraph { title, context });
    }

    Ok(corpus)
}

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
        let row: BeirQueryRow = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing query JSON on line {} from {}",
                idx + 1,
                path.display()
            )
        })?;
        queries.insert(
            row.id,
            BeirQuery {
                text: row.text.trim().to_string(),
            },
        );
    }

    Ok(queries)
}

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

    #[test]
    fn converts_basic_beir_layout() {
        let dir = tempdir().unwrap();
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

        let paragraphs = convert_beir(dir.path(), DatasetKind::Fever).unwrap();

        assert_eq!(paragraphs.len(), 2);
        let doc_one = paragraphs
            .iter()
            .find(|p| p.id == "fever-d1")
            .expect("missing paragraph for d1");
        assert_eq!(doc_one.questions.len(), 1);
        let question = &doc_one.questions[0];
        assert_eq!(question.id, "fever-q1");
        assert!(!question.answers.is_empty());
        assert!(doc_one.context.contains(&question.answers[0]));

        let doc_two = paragraphs
            .iter()
            .find(|p| p.id == "fever-d2")
            .expect("missing paragraph for d2");
        assert!(doc_two.questions.is_empty());
    }
}
