use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{TimeZone, Utc};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetKind {
    SquadV2,
    NaturalQuestions,
}

impl DatasetKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::SquadV2 => "squad-v2",
            Self::NaturalQuestions => "natural-questions-dev",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD v2.0",
            Self::NaturalQuestions => "Natural Questions (dev)",
        }
    }

    pub fn category(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD v2.0",
            Self::NaturalQuestions => "Natural Questions",
        }
    }

    pub fn entity_suffix(self) -> &'static str {
        match self {
            Self::SquadV2 => "SQuAD",
            Self::NaturalQuestions => "Natural Questions",
        }
    }

    pub fn source_prefix(self) -> &'static str {
        match self {
            Self::SquadV2 => "squad",
            Self::NaturalQuestions => "nq",
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
            other => {
                anyhow::bail!("unknown dataset '{other}'. Expected 'squad' or 'natural-questions'.")
            }
        }
    }
}

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
    pub generated_at: chrono::DateTime<Utc>,
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
        DatasetKind::SquadV2 => convert_squad(raw_path)?,
        DatasetKind::NaturalQuestions => {
            convert_nq(raw_path, include_unanswerable, context_token_limit)?
        }
    };

    let metadata_limit = match dataset {
        DatasetKind::NaturalQuestions => None,
        _ => context_token_limit,
    };

    Ok(ConvertedDataset {
        generated_at: Utc::now(),
        metadata: DatasetMetadata::for_kind(dataset, include_unanswerable, metadata_limit),
        source: raw_path.display().to_string(),
        paragraphs,
    })
}

fn convert_squad(raw_path: &Path) -> Result<Vec<ConvertedParagraph>> {
    #[derive(Debug, Deserialize)]
    struct SquadDataset {
        data: Vec<SquadArticle>,
    }

    #[derive(Debug, Deserialize)]
    struct SquadArticle {
        title: String,
        paragraphs: Vec<SquadParagraph>,
    }

    #[derive(Debug, Deserialize)]
    struct SquadParagraph {
        context: String,
        qas: Vec<SquadQuestion>,
    }

    #[derive(Debug, Deserialize)]
    struct SquadQuestion {
        id: String,
        question: String,
        answers: Vec<SquadAnswer>,
        #[serde(default)]
        is_impossible: bool,
    }

    #[derive(Debug, Deserialize)]
    struct SquadAnswer {
        text: String,
    }

    let raw = fs::read_to_string(raw_path)
        .with_context(|| format!("reading raw SQuAD dataset at {}", raw_path.display()))?;
    let parsed: SquadDataset = serde_json::from_str(&raw)
        .with_context(|| format!("parsing SQuAD dataset at {}", raw_path.display()))?;

    let mut paragraphs = Vec::new();
    for (article_idx, article) in parsed.data.into_iter().enumerate() {
        for (paragraph_idx, paragraph) in article.paragraphs.into_iter().enumerate() {
            let mut questions = Vec::new();
            for qa in paragraph.qas {
                let answers = dedupe_strings(qa.answers.into_iter().map(|answer| answer.text));
                questions.push(ConvertedQuestion {
                    id: qa.id,
                    question: qa.question.trim().to_string(),
                    answers,
                    is_impossible: qa.is_impossible,
                });
            }

            let paragraph_id =
                format!("{}-{}", slugify(&article.title, article_idx), paragraph_idx);

            paragraphs.push(ConvertedParagraph {
                id: paragraph_id,
                title: article.title.trim().to_string(),
                context: paragraph.context.trim().to_string(),
                questions,
            });
        }
    }

    Ok(paragraphs)
}

#[allow(dead_code)]
pub const DEFAULT_CONTEXT_TOKEN_LIMIT: usize = 1_500; // retained for backwards compatibility (unused)

fn convert_nq(
    raw_path: &Path,
    include_unanswerable: bool,
    _context_token_limit: Option<usize>,
) -> Result<Vec<ConvertedParagraph>> {
    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqExample {
        question_text: String,
        document_title: String,
        example_id: i64,
        document_tokens: Vec<NqToken>,
        long_answer_candidates: Vec<NqLongAnswerCandidate>,
        annotations: Vec<NqAnnotation>,
    }

    #[derive(Debug, Deserialize)]
    struct NqToken {
        token: String,
        #[serde(default)]
        html_token: bool,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqLongAnswerCandidate {
        start_token: i32,
        end_token: i32,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqAnnotation {
        short_answers: Vec<NqShortAnswer>,
        #[serde(default)]
        yes_no_answer: String,
        long_answer: NqLongAnswer,
    }

    #[derive(Debug, Deserialize)]
    struct NqShortAnswer {
        start_token: i32,
        end_token: i32,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqLongAnswer {
        candidate_index: i32,
    }

    fn join_tokens(tokens: &[NqToken], start: usize, end: usize) -> String {
        let mut buffer = String::new();
        let end = end.min(tokens.len());
        for token in tokens.iter().skip(start).take(end.saturating_sub(start)) {
            if token.html_token {
                continue;
            }
            let text = token.token.trim();
            if text.is_empty() {
                continue;
            }
            let attach = matches!(
                text,
                "," | "." | "!" | "?" | ";" | ":" | ")" | "]" | "}" | "%" | "…" | "..."
            ) || text.starts_with('\'')
                || text == "n't"
                || text == "'s"
                || text == "'re"
                || text == "'ve"
                || text == "'d"
                || text == "'ll";

            if buffer.is_empty() || attach {
                buffer.push_str(text);
            } else {
                buffer.push(' ');
                buffer.push_str(text);
            }
        }

        buffer.trim().to_string()
    }

    let file = File::open(raw_path).with_context(|| {
        format!(
            "opening Natural Questions dataset at {}",
            raw_path.display()
        )
    })?;
    let reader = BufReader::new(file);

    let mut paragraphs = Vec::new();
    for (line_idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "reading Natural Questions line {} from {}",
                line_idx + 1,
                raw_path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let example: NqExample = serde_json::from_str(&line).with_context(|| {
            format!(
                "parsing Natural Questions JSON (line {}) at {}",
                line_idx + 1,
                raw_path.display()
            )
        })?;

        let mut answer_texts: Vec<String> = Vec::new();
        let mut short_answer_texts: Vec<String> = Vec::new();
        let mut has_short_or_yesno = false;
        let mut has_short_answer = false;
        for annotation in &example.annotations {
            for short in &annotation.short_answers {
                if short.start_token < 0 || short.end_token <= short.start_token {
                    continue;
                }
                let start = short.start_token as usize;
                let end = short.end_token as usize;
                if start >= example.document_tokens.len() || end > example.document_tokens.len() {
                    continue;
                }
                let text = join_tokens(&example.document_tokens, start, end);
                if !text.is_empty() {
                    answer_texts.push(text.clone());
                    short_answer_texts.push(text);
                    has_short_or_yesno = true;
                    has_short_answer = true;
                }
            }

            match annotation
                .yes_no_answer
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "yes" => {
                    answer_texts.push("yes".to_string());
                    has_short_or_yesno = true;
                }
                "no" => {
                    answer_texts.push("no".to_string());
                    has_short_or_yesno = true;
                }
                _ => {}
            }
        }

        let mut answers = dedupe_strings(answer_texts);
        let is_unanswerable = !has_short_or_yesno || answers.is_empty();
        if is_unanswerable {
            if !include_unanswerable {
                continue;
            }
            answers.clear();
        }

        let paragraph_id = format!("nq-{}", example.example_id);
        let question_id = format!("nq-{}", example.example_id);

        let context = join_tokens(&example.document_tokens, 0, example.document_tokens.len());
        if context.is_empty() {
            continue;
        }

        if has_short_answer && !short_answer_texts.is_empty() {
            let normalized_context = context.to_ascii_lowercase();
            let missing_answer = short_answer_texts.iter().any(|answer| {
                let needle = answer.trim().to_ascii_lowercase();
                !needle.is_empty() && !normalized_context.contains(&needle)
            });
            if missing_answer {
                warn!(
                    question_id = %question_id,
                    "Skipping Natural Questions example because answers were not found in the assembled context"
                );
                continue;
            }
        }

        if !include_unanswerable && (!has_short_answer || short_answer_texts.is_empty()) {
            // yes/no-only questions are excluded by default unless --llm-mode is used
            continue;
        }

        let question = ConvertedQuestion {
            id: question_id,
            question: example.question_text.trim().to_string(),
            answers,
            is_impossible: is_unanswerable,
        };

        paragraphs.push(ConvertedParagraph {
            id: paragraph_id,
            title: example.document_title.trim().to_string(),
            context,
            questions: vec![question],
        });
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

fn dedupe_strings<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut set = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            set.insert(trimmed.to_string());
        }
    }
    set.into_iter().collect()
}

fn slugify(input: &str, fallback_idx: usize) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }

    slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        slug = format!("article-{fallback_idx}");
    }
    slug
}

pub fn base_timestamp() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .expect("valid base timestamp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn convert_nq_handles_answers_and_skips_unanswerable() {
        let mut file = NamedTempFile::new().expect("temp file");

        let record_with_short_answers = json!({
            "question_text": "What is foo?",
            "document_title": "Foo Title",
            "example_id": 123,
            "document_tokens": [
                {"token": "Foo", "html_token": false},
                {"token": "is", "html_token": false},
                {"token": "bar", "html_token": false},
                {"token": ".", "html_token": false}
            ],
            "long_answer_candidates": [
                {"start_token": 0, "end_token": 4, "top_level": true}
            ],
            "annotations": [
                {
                    "long_answer": {"start_token": 0, "end_token": 4, "candidate_index": 0},
                    "short_answers": [
                        {"start_token": 2, "end_token": 3},
                        {"start_token": 2, "end_token": 3}
                    ],
                    "yes_no_answer": "NONE"
                }
            ]
        });

        let record_with_yes_no = json!({
            "question_text": "Is bar real?",
            "document_title": "Bar Title",
            "example_id": 456,
            "document_tokens": [
                {"token": "Yes", "html_token": false},
                {"token": ",", "html_token": false},
                {"token": "bar", "html_token": false},
                {"token": "is", "html_token": false}
            ],
            "long_answer_candidates": [
                {"start_token": 0, "end_token": 4, "top_level": true}
            ],
            "annotations": [
                {
                    "long_answer": {"start_token": 0, "end_token": 4, "candidate_index": 0},
                    "short_answers": [],
                    "yes_no_answer": "YES"
                }
            ]
        });

        let unanswerable_record = json!({
            "question_text": "Unknown?",
            "document_title": "Unknown Title",
            "example_id": 789,
            "document_tokens": [
                {"token": "No", "html_token": false},
                {"token": "answer", "html_token": false}
            ],
            "long_answer_candidates": [
                {"start_token": 0, "end_token": 2, "top_level": true}
            ],
            "annotations": [
                {
                    "long_answer": {"start_token": 0, "end_token": 2, "candidate_index": 0},
                    "short_answers": [],
                    "yes_no_answer": "NONE"
                }
            ]
        });

        writeln!(file, "{}", record_with_short_answers).unwrap();
        writeln!(file, "{}", record_with_yes_no).unwrap();
        writeln!(file, "{}", unanswerable_record).unwrap();
        file.flush().unwrap();

        let dataset = convert(
            file.path(),
            DatasetKind::NaturalQuestions,
            false,
            Some(DEFAULT_CONTEXT_TOKEN_LIMIT),
        )
        .expect("convert natural questions");

        assert_eq!(dataset.metadata.id, DatasetKind::NaturalQuestions.id());
        assert!(!dataset.metadata.include_unanswerable);
        assert_eq!(dataset.paragraphs.len(), 2);

        let first = &dataset.paragraphs[0];
        assert_eq!(first.id, "nq-123");
        assert!(first.context.contains("Foo"));
        let first_answers = &first.questions.first().expect("question present").answers;
        assert_eq!(first_answers, &vec!["bar".to_string()]);

        let second = &dataset.paragraphs[1];
        assert_eq!(second.id, "nq-456");
        let second_answers = &second.questions.first().expect("question present").answers;
        assert_eq!(second_answers, &vec!["yes".to_string()]);

        assert!(dataset
            .paragraphs
            .iter()
            .all(|paragraph| paragraph.id != "nq-789"));
    }

    #[test]
    fn convert_nq_includes_unanswerable_when_flagged() {
        let mut file = NamedTempFile::new().expect("temp file");

        let answerable = json!({
            "question_text": "What is foo?",
            "document_title": "Foo Title",
            "example_id": 123,
            "document_tokens": [
                {"token": "Foo", "html_token": false},
                {"token": "is", "html_token": false},
                {"token": "bar", "html_token": false}
            ],
            "long_answer_candidates": [
                {"start_token": 0, "end_token": 3, "top_level": true}
            ],
            "annotations": [
                {
                    "long_answer": {"start_token": 0, "end_token": 3, "candidate_index": 0},
                    "short_answers": [
                        {"start_token": 2, "end_token": 3}
                    ],
                    "yes_no_answer": "NONE"
                }
            ]
        });

        let unanswerable = json!({
            "question_text": "Unknown?",
            "document_title": "Unknown Title",
            "example_id": 456,
            "document_tokens": [
                {"token": "No", "html_token": false},
                {"token": "answer", "html_token": false}
            ],
            "long_answer_candidates": [
                {"start_token": 0, "end_token": 2, "top_level": true}
            ],
            "annotations": [
                {
                    "long_answer": {"start_token": 0, "end_token": 2, "candidate_index": -1},
                    "short_answers": [],
                    "yes_no_answer": "NONE"
                }
            ]
        });

        writeln!(file, "{}", answerable).unwrap();
        writeln!(file, "{}", unanswerable).unwrap();
        file.flush().unwrap();

        let dataset = convert(
            file.path(),
            DatasetKind::NaturalQuestions,
            true,
            Some(DEFAULT_CONTEXT_TOKEN_LIMIT),
        )
        .expect("convert natural questions with unanswerable");

        assert!(dataset.metadata.include_unanswerable);
        assert_eq!(dataset.paragraphs.len(), 2);
        let impossible = dataset
            .paragraphs
            .iter()
            .find(|p| p.id == "nq-456")
            .expect("unanswerable paragraph present");
        let question = impossible.questions.first().expect("question present");
        assert!(question.answers.is_empty());
        assert!(question.is_impossible);
    }

    #[test]
    fn catalog_lists_datasets_and_slices() {
        let catalog = catalog().expect("catalog");
        let squad = catalog.dataset("squad-v2").expect("squad dataset");
        assert!(squad.raw_path.exists());
        assert!(squad.converted_path.exists());
        assert!(!squad.slices.is_empty());

        let (dataset, slice) = catalog.slice("squad-dev-200").expect("slice");
        assert_eq!(dataset.metadata.id, squad.metadata.id);
        assert_eq!(slice.dataset_id, squad.metadata.id);
        assert!(slice.limit.is_some());
    }
}
