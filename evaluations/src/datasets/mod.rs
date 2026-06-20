mod beir;
mod beir_mix;
mod checksum;
mod loader;
mod nq;
mod squad;
mod store;

use std::{
    collections::{BTreeMap, HashMap},
    fs::{self},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use clap::ValueEnum;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tracing::warn;

const MANIFEST_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/manifest.yaml");
static DATASET_CATALOG: OnceCell<DatasetCatalog> = OnceCell::new();

#[derive(Debug, Clone)]
pub struct DatasetCatalog {
    datasets: BTreeMap<String, DatasetEntry>,
    slices: HashMap<String, SliceLocation>,
}

#[derive(Debug, Clone)]
pub struct DatasetEntry {
    pub metadata: DatasetMetadata,
    pub raw_path: PathBuf,
    pub converted_path: PathBuf,
    pub slices: Vec<SliceEntry>,
}

#[derive(Debug, Clone)]
pub struct SliceEntry {
    pub id: String,
    pub dataset_id: String,
    pub limit: Option<usize>,
    pub corpus_limit: Option<usize>,
    pub include_unanswerable: Option<bool>,
    pub seed: Option<u64>,
    pub negative_multiplier: Option<f32>,
}

#[derive(Debug, Clone)]
struct SliceLocation {
    dataset_id: String,
    slice_index: usize,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    datasets: Vec<ManifestDataset>,
}

#[derive(Debug, Deserialize)]
struct ManifestDataset {
    id: String,
    label: String,
    category: String,
    #[serde(default)]
    entity_suffix: Option<String>,
    #[serde(default)]
    source_prefix: Option<String>,
    raw: String,
    converted: String,
    #[serde(default)]
    include_unanswerable: bool,
    #[serde(default)]
    slices: Vec<ManifestSlice>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ManifestSlice {
    id: String,
    label: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    corpus_limit: Option<usize>,
    #[serde(default)]
    include_unanswerable: Option<bool>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    negative_multiplier: Option<f32>,
}

impl DatasetCatalog {
    pub fn load() -> Result<Self> {
        let manifest_raw = fs::read_to_string(MANIFEST_PATH)
            .with_context(|| format!("reading dataset manifest at {MANIFEST_PATH}"))?;
        let manifest: ManifestFile = serde_yaml::from_str(&manifest_raw)
            .with_context(|| format!("parsing dataset manifest at {MANIFEST_PATH}"))?;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut datasets = BTreeMap::new();
        let mut slices = HashMap::new();

        for dataset in manifest.datasets {
            let raw_path = resolve_path(root, &dataset.raw);
            let converted_path = resolve_path(root, &dataset.converted);

            if !raw_path.exists() && dataset.id != "beir" {
                bail!(
                    "dataset '{}' raw file missing at {}",
                    dataset.id,
                    raw_path.display()
                );
            }
            let store_dir = store::store_dir_for(&converted_path);
            if !converted_path.exists() && !store_dir.join("meta.json").is_file() {
                warn!(
                    "dataset '{}' converted store missing at {}; the next conversion run will regenerate it",
                    dataset.id,
                    store_dir.display()
                );
            }

            let metadata = DatasetMetadata {
                id: dataset.id.clone(),
                label: dataset.label.clone(),
                category: dataset.category.clone(),
                entity_suffix: dataset
                    .entity_suffix
                    .clone()
                    .unwrap_or_else(|| dataset.label.clone()),
                source_prefix: dataset
                    .source_prefix
                    .clone()
                    .unwrap_or_else(|| dataset.id.clone()),
                include_unanswerable: dataset.include_unanswerable,
            };

            let mut entry_slices = Vec::with_capacity(dataset.slices.len());

            for (index, manifest_slice) in dataset.slices.into_iter().enumerate() {
                if slices.contains_key(&manifest_slice.id) {
                    bail!(
                        "slice '{}' defined multiple times in manifest",
                        manifest_slice.id
                    );
                }
                entry_slices.push(SliceEntry {
                    id: manifest_slice.id.clone(),
                    dataset_id: dataset.id.clone(),
                    limit: manifest_slice.limit,
                    corpus_limit: manifest_slice.corpus_limit,
                    include_unanswerable: manifest_slice.include_unanswerable,
                    seed: manifest_slice.seed,
                    negative_multiplier: manifest_slice.negative_multiplier,
                });
                slices.insert(
                    manifest_slice.id,
                    SliceLocation {
                        dataset_id: dataset.id.clone(),
                        slice_index: index,
                    },
                );
            }

            datasets.insert(
                metadata.id.clone(),
                DatasetEntry {
                    metadata,
                    raw_path,
                    converted_path,
                    slices: entry_slices,
                },
            );
        }

        if datasets.is_empty() {
            bail!("dataset manifest does not include any datasets");
        }

        Ok(Self { datasets, slices })
    }

    pub fn global() -> Result<&'static Self> {
        DATASET_CATALOG.get_or_try_init(Self::load)
    }

    pub fn dataset(&self, id: &str) -> Result<&DatasetEntry> {
        self.datasets
            .get(id)
            .ok_or_else(|| anyhow!("unknown dataset '{id}' in manifest"))
    }

    pub fn slice(&self, slice_id: &str) -> Result<(&DatasetEntry, &SliceEntry)> {
        let location = self
            .slices
            .get(slice_id)
            .ok_or_else(|| anyhow!("unknown slice '{slice_id}' in manifest"))?;
        let dataset = self
            .datasets
            .get(&location.dataset_id)
            .ok_or_else(|| anyhow!("slice '{slice_id}' references missing dataset"))?;
        let slice = dataset
            .slices
            .get(location.slice_index)
            .ok_or_else(|| anyhow!("slice index out of bounds for '{slice_id}'"))?;
        Ok((dataset, slice))
    }
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

pub use beir_mix::{beir_subset_store_summary, beir_subset_stores_ready, mix_content_checksum};
pub use checksum::store_aggregate_checksum;
pub use loader::{prebuild_catalog_slices, prepare_dataset};
pub use store::{
    content_checksum_for_layout, detect_layout, store_dir_for, write_sharded, ConvertedLayout,
};

pub fn catalog() -> Result<&'static DatasetCatalog> {
    DatasetCatalog::global()
}

pub(crate) fn dataset_entry_for_kind(kind: DatasetKind) -> Result<&'static DatasetEntry> {
    let catalog = catalog()?;
    catalog.dataset(kind.id())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Default)]
pub enum DatasetKind {
    SquadV2,
    NaturalQuestions,
    #[default]
    Beir,
    #[value(name = "fever")]
    Fever,
    #[value(name = "fiqa")]
    Fiqa,
    #[value(name = "hotpotqa", alias = "hotpot-qa")]
    HotpotQa,
    #[value(name = "nfcorpus", alias = "nf-corpus")]
    Nfcorpus,
    #[value(name = "quora")]
    Quora,
    #[value(name = "trec-covid", alias = "treccovid", alias = "trec_covid")]
    TrecCovid,
    #[value(name = "scifact")]
    Scifact,
    #[value(name = "nq-beir", alias = "natural-questions-beir")]
    NqBeir,
}

impl DatasetKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::SquadV2 => "squad-v2",
            Self::NaturalQuestions => "natural-questions-dev",
            Self::Beir => "beir",
            Self::Fever => "fever",
            Self::Fiqa => "fiqa",
            Self::HotpotQa => "hotpotqa",
            Self::Nfcorpus => "nfcorpus",
            Self::Quora => "quora",
            Self::TrecCovid => "trec-covid",
            Self::Scifact => "scifact",
            Self::NqBeir => "nq-beir",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD v2.0",
            Self::NaturalQuestions => "Natural Questions (dev)",
            Self::Beir => "BEIR mix",
            Self::Fever => "FEVER (BEIR)",
            Self::Fiqa => "FiQA-2018 (BEIR)",
            Self::HotpotQa => "HotpotQA (BEIR)",
            Self::Nfcorpus => "NFCorpus (BEIR)",
            Self::Quora => "Quora (IR)",
            Self::TrecCovid => "TREC-COVID (BEIR)",
            Self::Scifact => "SciFact (BEIR)",
            Self::NqBeir => "Natural Questions (BEIR)",
        }
    }

    pub fn category(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD v2.0",
            Self::NaturalQuestions | Self::NqBeir => "Natural Questions",
            Self::Beir => "BEIR",
            Self::Fever => "FEVER",
            Self::Fiqa => "FiQA-2018",
            Self::HotpotQa => "HotpotQA",
            Self::Nfcorpus => "NFCorpus",
            Self::Quora => "Quora",
            Self::TrecCovid => "TREC-COVID",
            Self::Scifact => "SciFact",
        }
    }

    pub fn entity_suffix(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD",
            Self::NaturalQuestions | Self::NqBeir => "Natural Questions",
            Self::Beir => "BEIR",
            Self::Fever => "FEVER",
            Self::Fiqa => "FiQA",
            Self::HotpotQa => "HotpotQA",
            Self::Nfcorpus => "NFCorpus",
            Self::Quora => "Quora",
            Self::TrecCovid => "TREC-COVID",
            Self::Scifact => "SciFact",
        }
    }

    pub fn source_prefix(self) -> &'static str {
        match self {
            Self::SquadV2 => "squad",
            Self::NaturalQuestions => "nq",
            Self::Beir => "beir",
            Self::Fever => "fever",
            Self::Fiqa => "fiqa",
            Self::HotpotQa => "hotpotqa",
            Self::Nfcorpus => "nfcorpus",
            Self::Quora => "quora",
            Self::TrecCovid => "trec-covid",
            Self::Scifact => "scifact",
            Self::NqBeir => "nq-beir",
        }
    }

    pub fn default_raw_path(self) -> PathBuf {
        #[allow(clippy::panic)]
        match dataset_entry_for_kind(self) {
            Ok(entry) => entry.raw_path.clone(),
            Err(err) => panic!("dataset manifest missing entry for {self:?}: {err}"),
        }
    }

    pub fn default_converted_path(self) -> PathBuf {
        #[allow(clippy::panic)]
        match dataset_entry_for_kind(self) {
            Ok(entry) => entry.converted_path.clone(),
            Err(err) => panic!("dataset manifest missing entry for {self:?}: {err}"),
        }
    }
}

impl std::fmt::Display for DatasetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id())
    }
}

impl FromStr for DatasetKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "squad" | "squad-v2" | "squad_v2" => Ok(Self::SquadV2),
            "nq" | "natural-questions" | "natural_questions" | "natural-questions-dev" => {
                Ok(Self::NaturalQuestions)
            }
            "beir" => Ok(Self::Beir),
            "fever" => Ok(Self::Fever),
            "fiqa" | "fiqa-2018" => Ok(Self::Fiqa),
            "hotpotqa" | "hotpot-qa" => Ok(Self::HotpotQa),
            "nfcorpus" | "nf-corpus" => Ok(Self::Nfcorpus),
            "quora" => Ok(Self::Quora),
            "trec-covid" | "treccovid" | "trec_covid" => Ok(Self::TrecCovid),
            "scifact" => Ok(Self::Scifact),
            "nq-beir" | "natural-questions-beir" => Ok(Self::NqBeir),
            other => {
                anyhow::bail!("unknown dataset '{other}'. Expected one of: squad, natural-questions, beir, fever, fiqa, hotpotqa, nfcorpus, quora, trec-covid, scifact, nq-beir.")
            }
        }
    }
}

pub const BEIR_DATASETS: [DatasetKind; 8] = [
    DatasetKind::Fever,
    DatasetKind::Fiqa,
    DatasetKind::HotpotQa,
    DatasetKind::Nfcorpus,
    DatasetKind::Quora,
    DatasetKind::TrecCovid,
    DatasetKind::Scifact,
    DatasetKind::NqBeir,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMetadata {
    pub id: String,
    pub label: String,
    pub category: String,
    pub entity_suffix: String,
    pub source_prefix: String,
    #[serde(default)]
    pub include_unanswerable: bool,
}

impl DatasetMetadata {
    pub fn for_kind(kind: DatasetKind, include_unanswerable: bool) -> Self {
        if let Ok(entry) = dataset_entry_for_kind(kind) {
            return Self {
                id: entry.metadata.id.clone(),
                label: entry.metadata.label.clone(),
                category: entry.metadata.category.clone(),
                entity_suffix: entry.metadata.entity_suffix.clone(),
                source_prefix: entry.metadata.source_prefix.clone(),
                include_unanswerable,
            };
        }

        Self {
            id: kind.id().to_string(),
            label: kind.label().to_string(),
            category: kind.category().to_string(),
            entity_suffix: kind.entity_suffix().to_string(),
            source_prefix: kind.source_prefix().to_string(),
            include_unanswerable,
        }
    }
}

fn default_metadata() -> DatasetMetadata {
    DatasetMetadata::for_kind(DatasetKind::default(), false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedDataset {
    pub generated_at: DateTime<Utc>,
    #[serde(default = "default_metadata")]
    pub metadata: DatasetMetadata,
    pub source: String,
    pub paragraphs: Vec<ConvertedParagraph>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedParagraph {
    pub id: String,
    pub title: String,
    pub context: String,
    pub questions: Vec<ConvertedQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedQuestion {
    pub id: String,
    pub question: String,
    pub answers: Vec<String>,
    pub is_impossible: bool,
}

pub fn convert(
    raw_path: &Path,
    dataset: DatasetKind,
    include_unanswerable: bool,
) -> Result<ConvertedDataset> {
    let paragraphs = match dataset {
        DatasetKind::SquadV2 => squad::convert_squad(raw_path)?,
        DatasetKind::NaturalQuestions => nq::convert_nq(raw_path, include_unanswerable)?,
        DatasetKind::Beir => {
            bail!(
                "BEIR mix is prepared via slice-first subset stores; use prepare_beir_mix instead of convert"
            );
        }
        DatasetKind::Fever
        | DatasetKind::Fiqa
        | DatasetKind::HotpotQa
        | DatasetKind::Nfcorpus
        | DatasetKind::Quora
        | DatasetKind::TrecCovid
        | DatasetKind::Scifact
        | DatasetKind::NqBeir => beir::convert_beir(raw_path, dataset)?,
    };

    let generated_at = match dataset {
        DatasetKind::Beir
        | DatasetKind::Fever
        | DatasetKind::Fiqa
        | DatasetKind::HotpotQa
        | DatasetKind::Nfcorpus
        | DatasetKind::Quora
        | DatasetKind::TrecCovid
        | DatasetKind::Scifact
        | DatasetKind::NqBeir => base_timestamp(),
        _ => Utc::now(),
    };

    let source_label = match dataset {
        DatasetKind::Beir => "beir-mix".to_string(),
        _ => raw_path.display().to_string(),
    };

    Ok(ConvertedDataset {
        generated_at,
        metadata: DatasetMetadata::for_kind(dataset, include_unanswerable),
        source: source_label,
        paragraphs,
    })
}

pub fn base_timestamp() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()
}
