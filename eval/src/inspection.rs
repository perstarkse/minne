use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use common::storage::{db::SurrealDbClient, types::text_chunk::TextChunk};

use crate::{args::Config, eval::connect_eval_db, ingest, snapshot::DbSnapshotState};

pub async fn inspect_question(config: &Config) -> Result<()> {
    let question_id = config
        .inspect_question
        .as_ref()
        .ok_or_else(|| anyhow!("--inspect-question is required for inspection mode"))?;
    let manifest_path = config
        .inspect_manifest
        .as_ref()
        .ok_or_else(|| anyhow!("--inspect-manifest must be provided for inspection mode"))?;

    let manifest = load_manifest(manifest_path)?;
    let chunk_lookup = build_chunk_lookup(&manifest);

    let question = manifest
        .questions
        .iter()
        .find(|q| q.question_id == *question_id)
        .ok_or_else(|| {
            anyhow!(
                "question '{}' not found in manifest {}",
                question_id,
                manifest_path.display()
            )
        })?;

    println!("Question: {}", question.question_text);
    println!("Answers: {:?}", question.answers);
    println!(
        "matching_chunk_ids ({}):",
        question.matching_chunk_ids.len()
    );

    let mut missing_in_manifest = Vec::new();
    for chunk_id in &question.matching_chunk_ids {
        if let Some(entry) = chunk_lookup.get(chunk_id) {
            println!(
                "  - {} (paragraph: {})\n    snippet: {}",
                chunk_id, entry.paragraph_title, entry.snippet
            );
        } else {
            println!("  - {} (missing from manifest)", chunk_id);
            missing_in_manifest.push(chunk_id.clone());
        }
    }

    if missing_in_manifest.is_empty() {
        println!("All matching_chunk_ids are present in the ingestion manifest");
    } else {
        println!(
            "Missing chunk IDs in manifest {}: {:?}",
            manifest_path.display(),
            missing_in_manifest
        );
    }

    let db_state_path = config
        .inspect_db_state
        .clone()
        .unwrap_or_else(|| default_state_path(config, &manifest));
    if let Some(state) = load_db_state(&db_state_path)? {
        if let (Some(ns), Some(db_name)) = (state.namespace.as_deref(), state.database.as_deref()) {
            match connect_eval_db(config, ns, db_name).await {
                Ok(db) => match verify_chunks_in_db(&db, &question.matching_chunk_ids).await? {
                    MissingChunks::None => println!(
                        "All matching_chunk_ids exist in namespace '{}', database '{}'",
                        ns, db_name
                    ),
                    MissingChunks::Missing(list) => println!(
                        "Missing chunks in namespace '{}', database '{}': {:?}",
                        ns, db_name, list
                    ),
                },
                Err(err) => {
                    println!(
                        "Failed to connect to SurrealDB namespace '{}' / database '{}': {err}",
                        ns, db_name
                    );
                }
            }
        } else {
            println!(
                "State file {} is missing namespace/database fields; skipping live DB validation",
                db_state_path.display()
            );
        }
    } else {
        println!(
            "State file {} not found; skipping live DB validation",
            db_state_path.display()
        );
    }

    Ok(())
}

struct ChunkEntry {
    paragraph_title: String,
    snippet: String,
}

fn load_manifest(path: &Path) -> Result<ingest::CorpusManifest> {
    let bytes =
        fs::read(path).with_context(|| format!("reading ingestion manifest {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing ingestion manifest {}", path.display()))
}

fn build_chunk_lookup(manifest: &ingest::CorpusManifest) -> HashMap<String, ChunkEntry> {
    let mut lookup = HashMap::new();
    for paragraph in &manifest.paragraphs {
        for chunk in &paragraph.chunks {
            let snippet = chunk
                .chunk
                .chars()
                .take(160)
                .collect::<String>()
                .replace('\n', " ");
            lookup.insert(
                chunk.id.clone(),
                ChunkEntry {
                    paragraph_title: paragraph.title.clone(),
                    snippet,
                },
            );
        }
    }
    lookup
}

fn default_state_path(config: &Config, manifest: &ingest::CorpusManifest) -> PathBuf {
    config
        .cache_dir
        .join("snapshots")
        .join(&manifest.metadata.dataset_id)
        .join(&manifest.metadata.slice_id)
        .join("db/state.json")
}

fn load_db_state(path: &Path) -> Result<Option<DbSnapshotState>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("reading db state {}", path.display()))?;
    let state = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing db state {}", path.display()))?;
    Ok(Some(state))
}

enum MissingChunks {
    None,
    Missing(Vec<String>),
}

async fn verify_chunks_in_db(db: &SurrealDbClient, chunk_ids: &[String]) -> Result<MissingChunks> {
    let mut missing = Vec::new();
    for chunk_id in chunk_ids {
        let exists = db
            .get_item::<TextChunk>(chunk_id)
            .await
            .with_context(|| format!("fetching text_chunk {}", chunk_id))?
            .is_some();
        if !exists {
            missing.push(chunk_id.clone());
        }
    }
    if missing.is_empty() {
        Ok(MissingChunks::None)
    } else {
        Ok(MissingChunks::Missing(missing))
    }
}
