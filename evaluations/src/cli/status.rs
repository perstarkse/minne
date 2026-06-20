#![allow(clippy::module_name_repetitions)]

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{
    args::Config,
    corpus::{self, CorpusCacheConfig},
    datasets::{
        beir_subset_store_summary, beir_subset_stores_ready, content_checksum_for_layout,
        detect_layout, mix_content_checksum, store_dir_for, ConvertedLayout, DatasetKind,
    },
    db::{connect_eval_db, default_database, default_namespace, namespace_has_corpus},
    slice::{self, ledger_target},
};

#[derive(Debug, Clone, Serialize)]
pub struct EvalStatus {
    pub dataset: String,
    pub slice: Option<String>,
    pub converted: ConvertedStatus,
    pub slice_ledger: SliceLedgerStatus,
    pub corpus_cache: CorpusCacheStatus,
    pub namespace: NamespaceStatus,
    pub query_ready: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConvertedStatus {
    pub layout: String,
    pub path: String,
    pub ready: bool,
    pub partial_load_eligible: bool,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SliceLedgerStatus {
    pub ready: bool,
    pub path: Option<String>,
    pub cases: Option<usize>,
    pub positives: Option<usize>,
    pub negatives: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorpusCacheStatus {
    pub ready: bool,
    pub path: Option<String>,
    pub manifest_present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamespaceStatus {
    pub namespace: String,
    pub database: String,
    pub seeded: bool,
    pub namespace_seed_recorded: bool,
}

#[allow(clippy::too_many_lines)]
pub async fn collect_status(config: &Config) -> Result<EvalStatus> {
    let mut notes = Vec::new();
    let is_beir_mix = config.dataset == DatasetKind::Beir;
    let converted_path = &config.converted_dataset_path;
    let layout = if is_beir_mix {
        ConvertedLayout::Missing
    } else {
        detect_layout(converted_path)
    };
    let layout_label = if is_beir_mix {
        "beir-mix-subset-stores"
    } else {
        match layout {
            ConvertedLayout::ShardedStore => "sharded-store",
            ConvertedLayout::Missing => "missing",
        }
    };

    let store_dir = store_dir_for(converted_path);
    let display_path = if is_beir_mix {
        beir_subset_store_summary()?
            .into_iter()
            .map(|(subset, paragraphs, questions)| {
                format!("{subset}-minne ({paragraphs} paragraphs, {questions} questions)")
            })
            .collect::<Vec<_>>()
            .join("; ")
    } else {
        store_dir.display().to_string()
    };

    let manifest_path = slice::cached_manifest_path(config);
    let slice_config = slice::slice_config_with_limit(config, ledger_target(config));
    let slice_manifest = manifest_path
        .as_ref()
        .and_then(|path| slice::read_manifest_if_exists(path).ok().flatten());

    let slice_ledger = SliceLedgerStatus {
        ready: slice_manifest
            .as_ref()
            .is_some_and(|manifest| slice::manifest_is_complete(manifest, &slice_config)),
        path: manifest_path
            .as_ref()
            .map(|path| path.display().to_string()),
        cases: slice_manifest.as_ref().map(|manifest| manifest.case_count),
        positives: slice_manifest
            .as_ref()
            .map(|manifest| manifest.positive_paragraphs),
        negatives: slice_manifest
            .as_ref()
            .map(|manifest| manifest.negative_paragraphs),
    };

    let beir_paragraph_ids = slice_manifest.as_ref().map(|manifest| {
        manifest
            .paragraphs
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<std::collections::HashSet<_>>()
    });

    let converted_ready = if is_beir_mix {
        slice_ledger.ready
            && beir_paragraph_ids
                .as_ref()
                .is_some_and(|ids| beir_subset_stores_ready(ids).unwrap_or(false))
    } else {
        layout == ConvertedLayout::ShardedStore
    };

    let checksum = if is_beir_mix {
        beir_paragraph_ids
            .as_ref()
            .and_then(|ids| mix_content_checksum(ids).ok())
    } else if layout == ConvertedLayout::ShardedStore {
        content_checksum_for_layout(converted_path).ok()
    } else {
        None
    };

    let partial_load_eligible = slice_ledger.ready && config.slice.is_some();

    let corpus_cache = if let Some(manifest) = slice_manifest.as_ref() {
        let cache_settings = CorpusCacheConfig::from(config);
        let base_dir = corpus::cached_corpus_dir(
            &cache_settings,
            config.dataset.id(),
            manifest.slice_id.as_str(),
        );
        let manifest_present = corpus::load_cached_manifest(&base_dir)?.is_some();
        CorpusCacheStatus {
            ready: manifest_present,
            path: Some(base_dir.display().to_string()),
            manifest_present,
        }
    } else {
        CorpusCacheStatus {
            ready: false,
            path: None,
            manifest_present: false,
        }
    };

    let namespace = config.database.db_namespace.clone().unwrap_or_else(|| {
        default_namespace(config.dataset.id(), config.limit, config.slice.as_deref())
    });
    let database = config
        .database
        .db_database
        .clone()
        .unwrap_or_else(default_database);

    let namespace_seed = corpus_cache.path.as_ref().and_then(|path| {
        corpus::load_cached_manifest(Path::new(path))
            .ok()
            .flatten()
            .and_then(|manifest| manifest.metadata.namespace_seed)
    });

    let (seeded, namespace_seed_recorded) =
        match connect_eval_db(config, &namespace, &database).await {
            Ok(db) => {
                let has_corpus = namespace_has_corpus(&db).await.unwrap_or(false);
                (has_corpus, namespace_seed.is_some())
            }
            Err(err) => {
                notes.push(format!("SurrealDB unavailable: {err}"));
                (false, false)
            }
        };

    let query_ready = converted_ready
        && slice_ledger.ready
        && corpus_cache.ready
        && seeded
        && namespace_seed_recorded;

    if !query_ready {
        notes.push("Run `cargo eval --warm --slice <id>` to prepare corpus and namespace.".into());
    }

    Ok(EvalStatus {
        dataset: config.dataset.id().to_string(),
        slice: config.slice.clone(),
        converted: ConvertedStatus {
            layout: layout_label.to_string(),
            path: display_path,
            ready: converted_ready,
            partial_load_eligible,
            checksum,
        },
        slice_ledger,
        corpus_cache,
        namespace: NamespaceStatus {
            namespace,
            database,
            seeded,
            namespace_seed_recorded,
        },
        query_ready,
        notes,
    })
}

pub fn print_status(status: &EvalStatus) {
    println!("Evaluation status for dataset `{}`", status.dataset);
    if let Some(slice) = &status.slice {
        println!("Slice: {slice}");
    }
    println!(
        "Converted: {} ({})",
        if status.converted.ready {
            "ready"
        } else {
            "missing"
        },
        status.converted.layout
    );
    println!("Converted path: {}", status.converted.path);
    if status.converted.partial_load_eligible {
        println!("Slice-first loading: eligible");
    }
    println!(
        "Slice ledger: {}",
        if status.slice_ledger.ready {
            format!(
                "ready ({} cases, {} positives, {} negatives)",
                status.slice_ledger.cases.unwrap_or(0),
                status.slice_ledger.positives.unwrap_or(0),
                status.slice_ledger.negatives.unwrap_or(0)
            )
        } else {
            "missing or incomplete".to_string()
        }
    );
    if let Some(path) = &status.slice_ledger.path {
        println!("Slice ledger path: {path}");
    }
    println!(
        "Corpus cache: {}",
        if status.corpus_cache.ready {
            "ready"
        } else {
            "missing"
        }
    );
    if let Some(path) = &status.corpus_cache.path {
        println!("Corpus cache path: {path}");
    }
    println!(
        "Namespace `{}` / `{}`: seeded={}, namespace_seed_recorded={}",
        status.namespace.namespace,
        status.namespace.database,
        status.namespace.seeded,
        status.namespace.namespace_seed_recorded
    );
    println!(
        "Query-ready: {}",
        if status.query_ready { "yes" } else { "no" }
    );
    for note in &status.notes {
        println!("Note: {note}");
    }
}

pub async fn warm(config: &Config) -> Result<()> {
    let loaded =
        crate::datasets::prepare_dataset(config.dataset, config).context("preparing dataset")?;
    crate::pipeline::warm_evaluation(&loaded.dataset, config, &loaded.content_checksum)
        .await
        .context("warming evaluation corpus and namespace")?;
    let status = collect_status(config).await?;
    print_status(&status);
    Ok(())
}

pub async fn ensure_query_ready(config: &Config) -> Result<()> {
    let status = collect_status(config).await?;
    if status.query_ready {
        return Ok(());
    }
    print_status(&status);
    anyhow::bail!(
        "evaluation is not query-ready; run `cargo eval --warm --slice {}` first",
        config.slice.as_deref().unwrap_or("<slice-id>")
    );
}
