//! Evaluation utilities module - re-exports from focused submodules.

// Re-export types from the root types module
pub use crate::types::*;

// Re-export from focused modules at crate root (crate-internal only)
pub(crate) use crate::cases::{cases_from_manifest, SeededCase};
pub(crate) use crate::namespace::{
    can_reuse_namespace, connect_eval_db, default_database, default_namespace, ensure_eval_user,
    record_namespace_state,
};
pub(crate) use crate::settings::{enforce_system_settings, load_or_init_system_settings};

use std::path::Path;

use anyhow::{Context, Result};
use common::storage::db::SurrealDbClient;
use tokio::io::AsyncWriteExt;
use tracing::info;

use crate::{
    args::{self, Config},
    datasets::ConvertedDataset,
    slice::{self},
};

/// Grow the slice ledger to contain the target number of cases.
pub async fn grow_slice(dataset: &ConvertedDataset, config: &Config) -> Result<()> {
    let ledger_limit = ledger_target(config);
    let slice_settings = slice::slice_config_with_limit(config, ledger_limit);
    let slice =
        slice::resolve_slice(dataset, &slice_settings).context("resolving dataset slice")?;
    info!(
        slice = slice.manifest.slice_id.as_str(),
        cases = slice.manifest.case_count,
        positives = slice.manifest.positive_paragraphs,
        negatives = slice.manifest.negative_paragraphs,
        total_paragraphs = slice.manifest.total_paragraphs,
        "Slice ledger ready"
    );
    println!(
        "Slice `{}` now contains {} questions ({} positives, {} negatives)",
        slice.manifest.slice_id,
        slice.manifest.case_count,
        slice.manifest.positive_paragraphs,
        slice.manifest.negative_paragraphs
    );
    Ok(())
}

pub(crate) fn ledger_target(config: &Config) -> Option<usize> {
    match (config.slice_grow, config.limit) {
        (Some(grow), Some(limit)) => Some(limit.max(grow)),
        (Some(grow), None) => Some(grow),
        (None, limit) => limit,
    }
}

pub(crate) async fn write_chunk_diagnostics(path: &Path, cases: &[CaseDiagnostics]) -> Result<()> {
    args::ensure_parent(path)?;
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("creating diagnostics file {}", path.display()))?;
    for case in cases {
        let line = serde_json::to_vec(case).context("serialising chunk diagnostics entry")?;
        file.write_all(&line).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}

pub(crate) async fn warm_hnsw_cache(db: &SurrealDbClient, dimension: usize) -> Result<()> {
    // Create a dummy embedding for cache warming
    let dummy_embedding: Vec<f32> = (0..dimension).map(|i| (i as f32).sin()).collect();

    info!("Warming HNSW caches with sample queries");

    // Warm up chunk embedding index - just query the embedding table to load HNSW index
    let _ = db
        .client
        .query(
            r#"SELECT chunk_id
               FROM text_chunk_embedding
               WHERE embedding <|1,1|> $embedding
               LIMIT 5"#,
        )
        .bind(("embedding", dummy_embedding.clone()))
        .await
        .context("warming text chunk HNSW cache")?;

    // Warm up entity embedding index
    let _ = db
        .client
        .query(
            r#"SELECT entity_id
               FROM knowledge_entity_embedding
               WHERE embedding <|1,1|> $embedding
               LIMIT 5"#,
        )
        .bind(("embedding", dummy_embedding))
        .await
        .context("warming knowledge entity HNSW cache")?;

    info!("HNSW cache warming completed");
    Ok(())
}

use chrono::{DateTime, SecondsFormat, Utc};

pub fn format_timestamp(timestamp: &DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn sanitize_model_code(code: &str) -> String {
    code.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

// Re-export run_evaluation from the pipeline module at crate root
pub use crate::pipeline::run_evaluation;
