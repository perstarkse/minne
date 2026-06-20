use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use tracing::info;

use super::{
    beir,
    checksum::hash_file,
    store::{
        self, build_dataset_from_catalog, paragraph_path, read_meta, store_dir_for,
        upsert_sharded_paragraphs, write_sharded,
    },
    ConvertedDataset, DatasetKind, DatasetMetadata, BEIR_DATASETS,
};
use crate::{args::Config, slice};

pub fn subset_for_paragraph_id(paragraph_id: &str) -> Option<DatasetKind> {
    let mut kinds: Vec<DatasetKind> = BEIR_DATASETS.to_vec();
    kinds.sort_by_key(|kind| std::cmp::Reverse(kind.source_prefix().len()));
    for kind in kinds {
        let prefix = format!("{}-", kind.source_prefix());
        if paragraph_id.starts_with(&prefix) {
            return Some(kind);
        }
    }
    None
}

pub fn build_beir_mix_qrels_dataset(include_unanswerable: bool) -> Result<ConvertedDataset> {
    if include_unanswerable {
        tracing::warn!("BEIR mix ignores include_unanswerable flag; all questions are answerable");
    }

    let mut paragraphs = Vec::new();
    for subset in BEIR_DATASETS {
        let entry = super::dataset_entry_for_kind(subset)?;
        let subset_paragraphs = beir::convert_beir(&entry.raw_path, subset)?;
        paragraphs.extend(subset_paragraphs);
    }

    Ok(ConvertedDataset {
        generated_at: super::base_timestamp(),
        metadata: DatasetMetadata::for_kind(DatasetKind::Beir, include_unanswerable),
        source: "beir-mix".to_string(),
        paragraphs,
    })
}

pub fn prepare_beir_mix(config: &Config) -> Result<super::loader::LoadedDataset> {
    let virtual_ds = build_beir_mix_qrels_dataset(config.llm_mode)?;
    let slice_config = slice::slice_config_with_limit(config, slice::ledger_target(config));
    let resolved = slice::resolve_slice(&virtual_ds, &slice_config)
        .context("resolving BEIR mix slice ledger (check --slice and --limit match your intent)")?;

    let unique: HashSet<String> = resolved
        .manifest
        .paragraphs
        .iter()
        .map(|entry| entry.id.clone())
        .collect();

    materialize_subset_stores(&unique, config.force_convert)?;

    let dataset = load_beir_mix_from_subsets(&unique)?;
    let checksum = mix_content_checksum(&unique)?;

    info!(
        slice = resolved.manifest.slice_id.as_str(),
        paragraphs = unique.len(),
        checksum = %checksum,
        "Prepared BEIR mix from per-subset converted stores"
    );

    Ok(super::loader::LoadedDataset {
        dataset,
        content_checksum: checksum,
        partial: true,
    })
}

pub fn materialize_subset_stores(paragraph_ids: &HashSet<String>, force: bool) -> Result<()> {
    let mut by_subset: HashMap<DatasetKind, Vec<String>> = HashMap::new();
    for paragraph_id in paragraph_ids {
        let kind = subset_for_paragraph_id(paragraph_id).with_context(|| {
            format!("routing BEIR mix paragraph id '{paragraph_id}' to subset store")
        })?;
        by_subset
            .entry(kind)
            .or_default()
            .push(paragraph_id.clone());
    }

    for (kind, ids) in by_subset {
        let entry = super::dataset_entry_for_kind(kind)?;
        let store_dir = store_dir_for(&entry.converted_path);
        let existing = if store_dir.join("meta.json").is_file() {
            store::load_paragraph_ids_set(&store_dir)?
        } else {
            HashSet::new()
        };

        let missing: Vec<String> = if force {
            ids
        } else {
            ids.into_iter()
                .filter(|paragraph_id| !existing.contains(paragraph_id))
                .collect()
        };

        if missing.is_empty() {
            continue;
        }

        let corpus_ids: HashSet<String> = missing
            .iter()
            .filter_map(|paragraph_id| beir::corpus_doc_id(paragraph_id, kind))
            .collect();
        let paragraphs = beir::convert_beir_documents(&entry.raw_path, kind, Some(&corpus_ids))?;

        if store_dir.join("meta.json").is_file() {
            upsert_sharded_paragraphs(&store_dir, &paragraphs)?;
        } else {
            let question_count = paragraphs
                .iter()
                .map(|paragraph| paragraph.questions.len())
                .sum::<usize>();
            let dataset = ConvertedDataset {
                generated_at: super::base_timestamp(),
                metadata: DatasetMetadata::for_kind(kind, false),
                source: entry.raw_path.display().to_string(),
                paragraphs,
            };
            write_sharded(&dataset, &store_dir)?;
            info!(
                subset = kind.id(),
                store = %store_dir.display(),
                paragraphs = dataset.paragraphs.len(),
                questions = question_count,
                "Created subset converted store for BEIR mix"
            );
        }
    }

    Ok(())
}

pub fn load_beir_mix_from_subsets(paragraph_ids: &HashSet<String>) -> Result<ConvertedDataset> {
    let mut by_subset: HashMap<DatasetKind, HashSet<String>> = HashMap::new();
    for paragraph_id in paragraph_ids {
        let kind = subset_for_paragraph_id(paragraph_id).with_context(|| {
            format!("routing BEIR mix paragraph id '{paragraph_id}' to subset store")
        })?;
        by_subset
            .entry(kind)
            .or_default()
            .insert(paragraph_id.clone());
    }

    let mut paragraphs = Vec::with_capacity(paragraph_ids.len());
    for (kind, subset_ids) in by_subset {
        let entry = super::dataset_entry_for_kind(kind)?;
        let store_dir = store_dir_for(&entry.converted_path);
        let partial = build_dataset_from_catalog(&store_dir, &subset_ids)?;
        paragraphs.extend(partial.paragraphs);
    }

    paragraphs.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(ConvertedDataset {
        generated_at: super::base_timestamp(),
        metadata: DatasetMetadata::for_kind(DatasetKind::Beir, false),
        source: "beir-mix".to_string(),
        paragraphs,
    })
}

pub fn mix_content_checksum(paragraph_ids: &HashSet<String>) -> Result<String> {
    let mut ids: Vec<String> = paragraph_ids.iter().cloned().collect();
    ids.sort();

    let mut hasher = Sha256::new();
    for paragraph_id in ids {
        let kind = subset_for_paragraph_id(&paragraph_id)
            .ok_or_else(|| anyhow!("unknown BEIR subset for paragraph '{paragraph_id}'"))?;
        let entry = super::dataset_entry_for_kind(kind)?;
        let store_dir = store_dir_for(&entry.converted_path);
        let path = paragraph_path(&store_dir, &paragraph_id);
        if !path.is_file() {
            return Err(anyhow!(
                "missing converted paragraph {} at {}",
                paragraph_id,
                path.display()
            ));
        }
        hasher.update(paragraph_id.as_bytes());
        hasher.update([0]);
        hasher.update(hash_file(&path)?.as_bytes());
    }

    Ok(format!("{:x}", hasher.finalize()))
}

pub fn beir_subset_stores_ready(paragraph_ids: &HashSet<String>) -> Result<bool> {
    for paragraph_id in paragraph_ids {
        let kind = subset_for_paragraph_id(paragraph_id).with_context(|| {
            format!("routing BEIR mix paragraph id '{paragraph_id}' to subset store")
        })?;
        let entry = super::dataset_entry_for_kind(kind)?;
        let store_dir = store_dir_for(&entry.converted_path);
        if !store_dir.join("meta.json").is_file() {
            return Ok(false);
        }
        if !paragraph_path(&store_dir, paragraph_id).is_file() {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn beir_subset_store_summary() -> Result<Vec<(String, usize, usize)>> {
    let mut summary = Vec::new();
    for kind in BEIR_DATASETS {
        let entry = super::dataset_entry_for_kind(kind)?;
        let store_dir = store_dir_for(&entry.converted_path);
        if store_dir.join("meta.json").is_file() {
            let meta = read_meta(&store_dir)?;
            summary.push((
                kind.id().to_string(),
                meta.paragraph_count,
                meta.question_count,
            ));
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_prefixed_paragraph_ids() {
        assert_eq!(
            subset_for_paragraph_id("fever-doc-1"),
            Some(DatasetKind::Fever)
        );
        assert_eq!(
            subset_for_paragraph_id("nq-beir-doc-1"),
            Some(DatasetKind::NqBeir)
        );
        assert_eq!(
            subset_for_paragraph_id("trec-covid-doc-1"),
            Some(DatasetKind::TrecCovid)
        );
        assert!(subset_for_paragraph_id("unknown-doc").is_none());
    }
}
