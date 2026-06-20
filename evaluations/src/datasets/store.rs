use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{
    checksum::store_aggregate_checksum, ConvertedDataset, ConvertedParagraph, ConvertedQuestion,
    DatasetMetadata,
};
use crate::slice;

pub const SHARDED_STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardedMeta {
    pub version: u32,
    pub generated_at: DateTime<Utc>,
    pub metadata: DatasetMetadata,
    pub source: String,
    pub paragraph_count: usize,
    pub question_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct QuestionRecord {
    paragraph_id: String,
    #[serde(flatten)]
    question: ConvertedQuestion,
}

#[derive(Debug, Clone)]
pub struct QuestionCatalog {
    pub entries: Vec<QuestionRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertedLayout {
    ShardedStore,
    Missing,
}

pub fn store_dir_for(converted_path: &Path) -> PathBuf {
    converted_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(converted_path.file_stem().map_or_else(
            || "dataset".to_string(),
            |stem| stem.to_string_lossy().into(),
        ))
}

pub fn detect_layout(converted_path: &Path) -> ConvertedLayout {
    let store_dir = store_dir_for(converted_path);
    if store_dir.join("meta.json").is_file() {
        ConvertedLayout::ShardedStore
    } else {
        ConvertedLayout::Missing
    }
}

fn paragraph_file_name(paragraph_id: &str) -> String {
    format!("{}.json", slice::paragraph_storage_key(paragraph_id))
}

pub fn paragraph_path(store_dir: &Path, paragraph_id: &str) -> PathBuf {
    store_dir
        .join("paragraphs")
        .join(paragraph_file_name(paragraph_id))
}

pub fn write_sharded(dataset: &ConvertedDataset, store_dir: &Path) -> Result<String> {
    if store_dir.exists() {
        fs::remove_dir_all(store_dir)
            .with_context(|| format!("clearing sharded store {}", store_dir.display()))?;
    }
    fs::create_dir_all(store_dir.join("paragraphs"))
        .with_context(|| format!("creating sharded store {}", store_dir.display()))?;

    let question_count = dataset
        .paragraphs
        .iter()
        .map(|paragraph| paragraph.questions.len())
        .sum::<usize>();

    let meta = ShardedMeta {
        version: SHARDED_STORE_VERSION,
        generated_at: dataset.generated_at,
        metadata: dataset.metadata.clone(),
        source: dataset.source.clone(),
        paragraph_count: dataset.paragraphs.len(),
        question_count,
    };
    let meta_path = store_dir.join("meta.json");
    fs::write(
        &meta_path,
        serde_json::to_vec_pretty(&meta).context("serialising sharded store metadata")?,
    )
    .with_context(|| format!("writing sharded metadata {}", meta_path.display()))?;

    let mut questions_file = File::create(store_dir.join("questions.jsonl"))
        .context("creating questions.jsonl for sharded store")?;
    let mut paragraph_ids_file = File::create(store_dir.join("paragraph_ids.jsonl"))
        .context("creating paragraph_ids.jsonl for sharded store")?;

    for paragraph in &dataset.paragraphs {
        writeln!(paragraph_ids_file, "{}", paragraph.id)
            .context("writing paragraph id to paragraph_ids.jsonl")?;
        for question in &paragraph.questions {
            let record = QuestionRecord {
                paragraph_id: paragraph.id.clone(),
                question: question.clone(),
            };
            serde_json::to_writer(&mut questions_file, &record)
                .context("writing question record to questions.jsonl")?;
            questions_file.write_all(b"\n")?;
        }

        let path = paragraph_path(store_dir, &paragraph.id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &path,
            serde_json::to_vec(paragraph).context("serialising sharded paragraph")?,
        )
        .with_context(|| format!("writing sharded paragraph {}", path.display()))?;
    }

    let digest = store_aggregate_checksum(store_dir)?;
    info!(
        store = %store_dir.display(),
        paragraphs = dataset.paragraphs.len(),
        questions = question_count,
        checksum = %digest,
        "Wrote sharded converted dataset"
    );
    Ok(digest)
}

pub fn read_meta(store_dir: &Path) -> Result<ShardedMeta> {
    let path = store_dir.join("meta.json");
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("reading sharded metadata {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("parsing sharded metadata {}", path.display()))
}

pub fn content_checksum_for_layout(converted_path: &Path) -> Result<String> {
    match detect_layout(converted_path) {
        ConvertedLayout::ShardedStore => {
            crate::datasets::store_aggregate_checksum(&store_dir_for(converted_path))
        }
        ConvertedLayout::Missing => Err(anyhow!(
            "converted dataset missing at {}",
            converted_path.display()
        )),
    }
}

fn load_paragraph(store_dir: &Path, paragraph_id: &str) -> Result<ConvertedParagraph> {
    let path = paragraph_path(store_dir, paragraph_id);
    let raw =
        fs::read(&path).with_context(|| format!("reading sharded paragraph {}", path.display()))?;
    serde_json::from_slice(&raw)
        .with_context(|| format!("parsing sharded paragraph {}", path.display()))
}

fn load_paragraphs(store_dir: &Path, paragraph_ids: &[String]) -> Result<Vec<ConvertedParagraph>> {
    paragraph_ids
        .iter()
        .map(|paragraph_id| load_paragraph(store_dir, paragraph_id))
        .collect()
}

pub fn load_sharded_partial(
    store_dir: &Path,
    paragraph_ids: &[String],
) -> Result<ConvertedDataset> {
    let meta = read_meta(store_dir)?;
    let mut paragraphs = load_paragraphs(store_dir, paragraph_ids)?;
    paragraphs.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ConvertedDataset {
        generated_at: meta.generated_at,
        metadata: meta.metadata,
        source: meta.source,
        paragraphs,
    })
}

pub fn load_sharded_full(store_dir: &Path) -> Result<ConvertedDataset> {
    let meta = read_meta(store_dir)?;
    let ids = load_paragraph_ids(store_dir)?;
    let paragraphs = load_paragraphs(store_dir, &ids)?;
    Ok(ConvertedDataset {
        generated_at: meta.generated_at,
        metadata: meta.metadata,
        source: meta.source,
        paragraphs,
    })
}

pub fn load_paragraph_ids_set(store_dir: &Path) -> Result<HashSet<String>> {
    Ok(load_paragraph_ids(store_dir)?.into_iter().collect())
}

#[allow(clippy::arithmetic_side_effects)]
pub fn upsert_sharded_paragraphs(
    store_dir: &Path,
    paragraphs: &[ConvertedParagraph],
) -> Result<()> {
    if paragraphs.is_empty() {
        return Ok(());
    }
    if !store_dir.join("meta.json").is_file() {
        return Err(anyhow!(
            "cannot upsert into missing sharded store at {}",
            store_dir.display()
        ));
    }

    fs::create_dir_all(store_dir.join("paragraphs"))
        .with_context(|| format!("creating paragraphs directory in {}", store_dir.display()))?;

    let existing = load_paragraph_ids_set(store_dir)?;
    let questions_path = store_dir.join("questions.jsonl");
    let mut questions_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&questions_path)
        .with_context(|| format!("opening question catalog {}", questions_path.display()))?;

    let mut ids_file = None;
    let mut new_paragraphs = 0usize;
    let mut new_questions = 0usize;

    for paragraph in paragraphs {
        let is_new = !existing.contains(&paragraph.id);
        let path = paragraph_path(store_dir, &paragraph.id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &path,
            serde_json::to_vec(paragraph).context("serialising sharded paragraph")?,
        )
        .with_context(|| format!("writing sharded paragraph {}", path.display()))?;

        if is_new {
            if ids_file.is_none() {
                ids_file = Some(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(store_dir.join("paragraph_ids.jsonl"))
                        .context("opening paragraph_ids.jsonl for append")?,
                );
            }
            if let Some(file) = ids_file.as_mut() {
                writeln!(file, "{}", paragraph.id).context("appending paragraph id")?;
            }
            new_paragraphs += 1;

            for question in &paragraph.questions {
                let record = QuestionRecord {
                    paragraph_id: paragraph.id.clone(),
                    question: question.clone(),
                };
                serde_json::to_writer(&mut questions_file, &record)
                    .context("writing question record to questions.jsonl")?;
                questions_file.write_all(b"\n")?;
                new_questions += 1;
            }
        }
    }

    if new_paragraphs > 0 || new_questions > 0 {
        let meta = read_meta(store_dir)?;
        let updated = ShardedMeta {
            paragraph_count: meta.paragraph_count + new_paragraphs,
            question_count: meta.question_count + new_questions,
            ..meta
        };
        fs::write(
            store_dir.join("meta.json"),
            serde_json::to_vec_pretty(&updated).context("serialising updated sharded metadata")?,
        )?;
        store_aggregate_checksum(store_dir)?;
        info!(
            store = %store_dir.display(),
            new_paragraphs,
            new_questions,
            "Upserted paragraphs into sharded converted store"
        );
    }

    Ok(())
}

pub fn load_paragraph_ids(store_dir: &Path) -> Result<Vec<String>> {
    let path = store_dir.join("paragraph_ids.jsonl");
    let file = File::open(&path)
        .with_context(|| format!("opening paragraph id index {}", path.display()))?;
    let reader = BufReader::new(file);
    reader
        .lines()
        .map(|line| {
            line.context("reading paragraph id index line")
                .and_then(|value| {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        Err(anyhow!("empty paragraph id in index"))
                    } else {
                        Ok(trimmed.to_string())
                    }
                })
        })
        .collect()
}

pub fn load_question_catalog(store_dir: &Path) -> Result<QuestionCatalog> {
    let path = store_dir.join("questions.jsonl");
    let file = File::open(&path)
        .with_context(|| format!("opening question catalog {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line.context("reading question catalog line")?;
        if line.trim().is_empty() {
            continue;
        }
        let record: QuestionRecord =
            serde_json::from_str(&line).context("parsing question catalog record")?;
        entries.push(record);
    }
    Ok(QuestionCatalog { entries })
}

pub fn build_dataset_from_catalog(
    store_dir: &Path,
    paragraph_ids: &HashSet<String>,
) -> Result<ConvertedDataset> {
    let catalog = load_question_catalog(store_dir)?;
    let mut questions_by_paragraph: HashMap<String, Vec<ConvertedQuestion>> = HashMap::new();
    for entry in catalog.entries {
        if paragraph_ids.contains(&entry.paragraph_id) {
            questions_by_paragraph
                .entry(entry.paragraph_id.clone())
                .or_default()
                .push(entry.question);
        }
    }

    let mut dataset = load_sharded_partial(
        store_dir,
        &paragraph_ids.iter().cloned().collect::<Vec<_>>(),
    )?;
    for paragraph in &mut dataset.paragraphs {
        if let Some(questions) = questions_by_paragraph.remove(&paragraph.id) {
            paragraph.questions = questions;
        } else {
            paragraph.questions.clear();
        }
    }

    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::{DatasetKind, DatasetMetadata};

    fn sample_dataset() -> ConvertedDataset {
        ConvertedDataset {
            generated_at: Utc::now(),
            metadata: DatasetMetadata::for_kind(DatasetKind::SquadV2, false),
            source: "test".to_string(),
            paragraphs: vec![ConvertedParagraph {
                id: "p1".to_string(),
                title: "Title".to_string(),
                context: "Body".to_string(),
                questions: vec![ConvertedQuestion {
                    id: "q1".to_string(),
                    question: "Question?".to_string(),
                    answers: vec!["Answer".to_string()],
                    is_impossible: false,
                }],
            }],
        }
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn sharded_round_trip() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store_dir = dir.path().join("sample");
        let dataset = sample_dataset();
        write_sharded(&dataset, &store_dir)?;

        let loaded = load_sharded_full(&store_dir)?;
        assert_eq!(loaded.paragraphs.len(), 1);
        assert_eq!(loaded.paragraphs[0].questions[0].id, "q1");
        Ok(())
    }
}
