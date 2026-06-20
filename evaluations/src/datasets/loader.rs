use std::collections::HashSet;

use anyhow::{Context, Result};
use tracing::info;

use super::{
    catalog,
    store::{
        self, build_dataset_from_catalog, detect_layout, read_meta, store_dir_for, write_sharded,
        ConvertedLayout,
    },
    ConvertedDataset, DatasetKind,
};
use crate::{
    args::Config,
    slice::{self, SliceConfig},
};

#[derive(Debug, Clone)]
pub struct LoadedDataset {
    pub dataset: ConvertedDataset,
    pub content_checksum: String,
    pub partial: bool,
}

pub fn prepare_dataset(dataset_kind: DatasetKind, config: &Config) -> Result<LoadedDataset> {
    if dataset_kind == DatasetKind::Beir {
        return super::beir_mix::prepare_beir_mix(config);
    }

    let converted_path = &config.converted_dataset_path;
    let layout = detect_layout(converted_path);
    let store_dir = store_dir_for(converted_path);

    if layout == ConvertedLayout::Missing || config.force_convert {
        return convert_and_load(dataset_kind, config);
    }

    load_from_store(dataset_kind, config, &store_dir, true)
}

fn convert_and_load(dataset_kind: DatasetKind, config: &Config) -> Result<LoadedDataset> {
    let dataset = super::convert(
        config.raw_dataset_path.as_path(),
        dataset_kind,
        config.llm_mode,
    )
    .with_context(|| format!("converting {} dataset", dataset_kind.label()))?;

    let store_dir = store_dir_for(&config.converted_dataset_path);
    write_sharded(&dataset, &store_dir)?;
    prebuild_catalog_slices(&dataset, config)?;
    let checksum = crate::datasets::store_aggregate_checksum(&store_dir)?;

    Ok(LoadedDataset {
        dataset,
        content_checksum: checksum,
        partial: false,
    })
}

fn load_from_store(
    dataset_kind: DatasetKind,
    config: &Config,
    store_dir: &std::path::Path,
    allow_partial: bool,
) -> Result<LoadedDataset> {
    let checksum = crate::datasets::store_aggregate_checksum(store_dir)?;
    let meta = read_meta(store_dir)?;
    validate_metadata_fields(&meta.metadata, dataset_kind, config)?;

    if allow_partial {
        if let Some(paragraph_ids) = slice_paragraph_ids_for_fast_path(config)? {
            let unique: HashSet<String> = paragraph_ids.into_iter().collect();
            info!(
                paragraphs = unique.len(),
                store = %store_dir.display(),
                "Loading slice-addressed paragraphs from sharded converted store"
            );
            let dataset = build_dataset_from_catalog(store_dir, &unique)?;
            return Ok(LoadedDataset {
                dataset,
                content_checksum: checksum,
                partial: true,
            });
        }
    }

    info!(
        store = %store_dir.display(),
        paragraphs = meta.paragraph_count,
        "Loading full sharded converted store"
    );
    let dataset = store::load_sharded_full(store_dir)?;
    Ok(LoadedDataset {
        dataset,
        content_checksum: checksum,
        partial: false,
    })
}

fn slice_paragraph_ids_for_fast_path(config: &Config) -> Result<Option<Vec<String>>> {
    let Some(manifest_path) = slice::cached_manifest_path(config) else {
        return Ok(None);
    };
    let Some(manifest) = slice::read_manifest_if_exists(&manifest_path)? else {
        return Ok(None);
    };
    let slice_config = slice::slice_config_with_limit(config, slice::ledger_target(config));
    if !slice::manifest_is_complete(&manifest, &slice_config) {
        return Ok(None);
    }
    Ok(Some(
        manifest
            .paragraphs
            .iter()
            .map(|entry| entry.id.clone())
            .collect(),
    ))
}

fn validate_metadata_fields(
    metadata: &super::DatasetMetadata,
    dataset_kind: DatasetKind,
    config: &Config,
) -> Result<()> {
    if metadata.id != dataset_kind.id() {
        anyhow::bail!(
            "converted dataset targets '{}', expected '{}'",
            metadata.id,
            dataset_kind.id()
        );
    }
    if metadata.include_unanswerable != config.llm_mode {
        anyhow::bail!(
            "converted dataset include_unanswerable mismatch (expected {}, found {})",
            config.llm_mode,
            metadata.include_unanswerable
        );
    }
    Ok(())
}

pub fn prebuild_catalog_slices(dataset: &ConvertedDataset, config: &Config) -> Result<()> {
    let catalog = catalog()?;
    let entry = catalog.dataset(dataset.metadata.id.as_str())?;
    if entry.slices.is_empty() {
        return Ok(());
    }

    info!(
        dataset = dataset.metadata.id.as_str(),
        slices = entry.slices.len(),
        "Prebuilding catalog slice ledgers"
    );

    for slice_entry in &entry.slices {
        let slice_config = slice_config_for_catalog_entry(config, slice_entry);
        match slice::resolve_slice(dataset, &slice_config) {
            Ok(resolved) => info!(
                slice = resolved.manifest.slice_id.as_str(),
                cases = resolved.manifest.case_count,
                positives = resolved.manifest.positive_paragraphs,
                negatives = resolved.manifest.negative_paragraphs,
                "Prebuilt catalog slice ledger"
            ),
            Err(err) => tracing::warn!(
                slice = slice_entry.id.as_str(),
                error = %err,
                "Failed to prebuild catalog slice ledger"
            ),
        }
    }

    Ok(())
}

fn slice_config_for_catalog_entry<'a>(
    config: &'a Config,
    slice_entry: &'a super::SliceEntry,
) -> SliceConfig<'a> {
    SliceConfig {
        cache_dir: config.cache_dir.as_path(),
        force_convert: config.force_convert,
        explicit_slice: Some(slice_entry.id.as_str()),
        limit: slice_entry.limit,
        corpus_limit: slice_entry.corpus_limit,
        slice_seed: slice_entry.seed.unwrap_or(config.slice_seed),
        llm_mode: slice_entry.include_unanswerable.unwrap_or(config.llm_mode),
        negative_multiplier: slice_entry
            .negative_multiplier
            .unwrap_or(config.negative_multiplier),
        require_verified_chunks: config.retrieval.require_verified_chunks,
    }
}
