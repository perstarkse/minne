mod beir;
mod nq;
mod squad;

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
#[allow(dead_code)]
pub struct DatasetCatalog {
    datasets: BTreeMap<String, DatasetEntry>,
    slices: HashMap<String, SliceLocation>,
    default_dataset: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DatasetEntry {
    pub metadata: DatasetMetadata,
    pub raw_path: PathBuf,
    pub converted_path: PathBuf,
    pub include_unanswerable: bool,
    pub slices: Vec<SliceEntry>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SliceEntry {
    pub id: String,
    pub dataset_id: String,
    pub label: String,
    pub description: Option<String>,
    pub limit: Option<usize>,
    pub corpus_limit: Option<usize>,
    pub include_unanswerable: Option<bool>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SliceLocation {
    dataset_id: String,
    slice_index: usize,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    default_dataset: Option<String>,
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
}

impl DatasetCatalog {
    pub fn load() -> Result<Self> {
        let manifest_raw = fs::read_to_string(MANIFEST_PATH)
            .with_context(|| format!("reading dataset manifest at {}", MANIFEST_PATH))?;
        let manifest: ManifestFile = serde_yaml::from_str(&manifest_raw)
            .with_context(|| format!("parsing dataset manifest at {}", MANIFEST_PATH))?;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut datasets = BTreeMap::new();
        let mut slices = HashMap::new();

        for dataset in manifest.datasets {
            let raw_path = resolve_path(root, &dataset.raw);
            let converted_path = resolve_path(root, &dataset.converted);

            if !raw_path.exists() {
                bail!(
                    "dataset '{}' raw file missing at {}",
                    dataset.id,
                    raw_path.display()
                );
            }
            if !converted_path.exists() {
                warn!(
                    "dataset '{}' converted file missing at {}; the next conversion run will regenerate it",
                    dataset.id,
                    converted_path.display()
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
                context_token_limit: None,
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
                    label: manifest_slice.label,
                    description: manifest_slice.description,
                    limit: manifest_slice.limit,
                    corpus_limit: manifest_slice.corpus_limit,
                    include_unanswerable: manifest_slice.include_unanswerable,
                    seed: manifest_slice.seed,
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
                    include_unanswerable: dataset.include_unanswerable,
                    slices: entry_slices,
                },
            );
        }

        let default_dataset = manifest
            .default_dataset
            .or_else(|| datasets.keys().next().cloned())
            .ok_or_else(|| anyhow!("dataset manifest does not include any datasets"))?;

        Ok(Self {
            datasets,
            slices,
            default_dataset,
        })
    }

    pub fn global() -> Result<&'static Self> {
        DATASET_CATALOG.get_or_try_init(Self::load)
    }

    pub fn dataset(&self, id: &str) -> Result<&DatasetEntry> {
        self.datasets
            .get(id)
            .ok_or_else(|| anyhow!("unknown dataset '{id}' in manifest"))
    }

    #[allow(dead_code)]
    pub fn default_dataset(&self) -> Result<&DatasetEntry> {
        self.dataset(&self.default_dataset)
    }

    #[allow(dead_code)]
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

pub fn catalog() -> Result<&'static DatasetCatalog> {
    DatasetCatalog::global()
}

fn dataset_entry_for_kind(kind: DatasetKind) -> Result<&'static DatasetEntry> {
    let catalog = catalog()?;
    catalog.dataset(kind.id())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DatasetKind {
    SquadV2,
    NaturalQuestions,
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
        }
    }

    pub fn category(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD v2.0",
            Self::NaturalQuestions => "Natural Questions",
            Self::Beir => "BEIR",
            Self::Fever => "FEVER",
            Self::Fiqa => "FiQA-2018",
            Self::HotpotQa => "HotpotQA",
            Self::Nfcorpus => "NFCorpus",
            Self::Quora => "Quora",
            Self::TrecCovid => "TREC-COVID",
        }
    }

    pub fn entity_suffix(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD",
            Self::NaturalQuestions => "Natural Questions",
            Self::Beir => "BEIR",
            Self::Fever => "FEVER",
            Self::Fiqa => "FiQA",
            Self::HotpotQa => "HotpotQA",
            Self::Nfcorpus => "NFCorpus",
            Self::Quora => "Quora",
            Self::TrecCovid => "TREC-COVID",
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
        }
    }

    pub fn default_raw_path(self) -> PathBuf {
        dataset_entry_for_kind(self)
            .map(|entry| entry.raw_path.clone())
            .unwrap_or_else(|err| panic!("dataset manifest missing entry for {:?}: {err}", self))
    }

    pub fn default_converted_path(self) -> PathBuf {
        dataset_entry_for_kind(self)
            .map(|entry| entry.converted_path.clone())
            .unwrap_or_else(|err| panic!("dataset manifest missing entry for {:?}: {err}", self))
    }
}

impl std::fmt::Display for DatasetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id())
    }
}

impl Default for DatasetKind {
    fn default() -> Self {
        Self::SquadV2
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
            other => {
                anyhow::bail!("unknown dataset '{other}'. Expected one of: squad, natural-questions, beir, fever, fiqa, hotpotqa, nfcorpus, quora, trec-covid.")
            }
        }
    }
}

pub const BEIR_DATASETS: [DatasetKind; 6] = [
    DatasetKind::Fever,
    DatasetKind::Fiqa,
    DatasetKind::HotpotQa,
    DatasetKind::Nfcorpus,
    DatasetKind::Quora,
    DatasetKind::TrecCovid,
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
    #[serde(default)]
    pub context_token_limit: Option<usize>,
}

impl DatasetMetadata {
    pub fn for_kind(
        kind: DatasetKind,
        include_unanswerable: bool,
        context_token_limit: Option<usize>,
    ) -> Self {
        if let Ok(entry) = dataset_entry_for_kind(kind) {
            return Self {
                id: entry.metadata.id.clone(),
                label: entry.metadata.label.clone(),
                category: entry.metadata.category.clone(),
                entity_suffix: entry.metadata.entity_suffix.clone(),
                source_prefix: entry.metadata.source_prefix.clone(),
                include_unanswerable,
                context_token_limit,
            };
        }

        Self {
            id: kind.id().to_string(),
            label: kind.label().to_string(),
            category: kind.category().to_string(),
            entity_suffix: kind.entity_suffix().to_string(),
            source_prefix: kind.source_prefix().to_string(),
            include_unanswerable,
            context_token_limit,
        }
    }
}

fn default_metadata() -> DatasetMetadata {
    DatasetMetadata::for_kind(DatasetKind::default(), false, None)
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
    context_token_limit: Option<usize>,
) -> Result<ConvertedDataset> {
    let paragraphs = match dataset {
        DatasetKind::SquadV2 => squad::convert_squad(raw_path)?,
        DatasetKind::NaturalQuestions => {
            nq::convert_nq(raw_path, include_unanswerable, context_token_limit)?
        }
        DatasetKind::Beir => convert_beir_mix(include_unanswerable, context_token_limit)?,
        DatasetKind::Fever
        | DatasetKind::Fiqa
        | DatasetKind::HotpotQa
        | DatasetKind::Nfcorpus
        | DatasetKind::Quora
        | DatasetKind::TrecCovid => beir::convert_beir(raw_path, dataset)?,
    };

    let metadata_limit = match dataset {
        DatasetKind::NaturalQuestions => None,
        _ => context_token_limit,
    };

    let source_label = match dataset {
        DatasetKind::Beir => "beir-mix".to_string(),
        _ => raw_path.display().to_string(),
    };

    Ok(ConvertedDataset {
        generated_at: Utc::now(),
        metadata: DatasetMetadata::for_kind(dataset, include_unanswerable, metadata_limit),
        source: source_label,
        paragraphs,
    })
}

fn convert_beir_mix(
    include_unanswerable: bool,
    _context_token_limit: Option<usize>,
) -> Result<Vec<ConvertedParagraph>> {
    if include_unanswerable {
        warn!("BEIR mix ignores include_unanswerable flag; all questions are answerable");
    }

    let mut paragraphs = Vec::new();
    for subset in BEIR_DATASETS {
        let entry = dataset_entry_for_kind(subset)?;
        let subset_paragraphs = beir::convert_beir(&entry.raw_path, subset)?;
        paragraphs.extend(subset_paragraphs);
    }

    Ok(paragraphs)
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory for {}", path.display()))?;
    }
    Ok(())
}

pub fn write_converted(dataset: &ConvertedDataset, converted_path: &Path) -> Result<()> {
    ensure_parent(converted_path)?;
    let json =
        serde_json::to_string_pretty(dataset).context("serialising converted dataset to JSON")?;
    fs::write(converted_path, json)
        .with_context(|| format!("writing converted dataset to {}", converted_path.display()))
}

pub fn read_converted(converted_path: &Path) -> Result<ConvertedDataset> {
    let raw = fs::read_to_string(converted_path)
        .with_context(|| format!("reading converted dataset at {}", converted_path.display()))?;
    let mut dataset: ConvertedDataset = serde_json::from_str(&raw)
        .with_context(|| format!("parsing converted dataset at {}", converted_path.display()))?;
    if dataset.metadata.id.trim().is_empty() {
        dataset.metadata = default_metadata();
    }
    if dataset.source.is_empty() {
        dataset.source = converted_path.display().to_string();
    }
    Ok(dataset)
}

pub fn ensure_converted(
    dataset_kind: DatasetKind,
    raw_path: &Path,
    converted_path: &Path,
    force: bool,
    include_unanswerable: bool,
    context_token_limit: Option<usize>,
) -> Result<ConvertedDataset> {
    if force || !converted_path.exists() {
        let dataset = convert(
            raw_path,
            dataset_kind,
            include_unanswerable,
            context_token_limit,
        )?;
        write_converted(&dataset, converted_path)?;
        return Ok(dataset);
    }

    match read_converted(converted_path) {
        Ok(dataset)
            if dataset.metadata.id == dataset_kind.id()
                && dataset.metadata.include_unanswerable == include_unanswerable
                && dataset.metadata.context_token_limit == context_token_limit =>
        {
            Ok(dataset)
        }
        _ => {
            let dataset = convert(
                raw_path,
                dataset_kind,
                include_unanswerable,
                context_token_limit,
            )?;
            write_converted(&dataset, converted_path)?;
            Ok(dataset)
        }
    }
}

pub fn base_timestamp() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()
}
