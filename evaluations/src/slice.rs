use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::datasets::{
    ConvertedDataset, ConvertedParagraph, ConvertedQuestion, DatasetKind, BEIR_DATASETS,
};

const SLICE_VERSION: u32 = 2;
pub const DEFAULT_NEGATIVE_MULTIPLIER: f32 = 4.0;

#[derive(Debug, Clone)]
pub struct SliceConfig<'a> {
    pub cache_dir: &'a Path,
    pub force_convert: bool,
    pub explicit_slice: Option<&'a str>,
    pub limit: Option<usize>,
    pub corpus_limit: Option<usize>,
    pub slice_seed: u64,
    pub llm_mode: bool,
    pub negative_multiplier: f32,
    pub require_verified_chunks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceManifest {
    pub version: u32,
    pub slice_id: String,
    pub dataset_id: String,
    pub dataset_label: String,
    pub dataset_source: String,
    pub includes_unanswerable: bool,
    #[serde(default = "default_require_verified_chunks")]
    pub require_verified_chunks: bool,
    pub seed: u64,
    pub requested_limit: Option<usize>,
    pub requested_corpus: usize,
    pub generated_at: DateTime<Utc>,
    pub case_count: usize,
    pub positive_paragraphs: usize,
    pub negative_paragraphs: usize,
    pub total_paragraphs: usize,
    pub negative_multiplier: f32,
    pub cases: Vec<SliceCaseEntry>,
    pub paragraphs: Vec<SliceParagraphEntry>,
}

fn default_require_verified_chunks() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceCaseEntry {
    pub question_id: String,
    pub paragraph_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceParagraphEntry {
    pub id: String,
    pub kind: SliceParagraphKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SliceParagraphKind {
    Positive { question_ids: Vec<String> },
    Negative,
}

pub(crate) fn default_shard_path(paragraph_id: &str) -> String {
    let sanitized = sanitize_identifier(paragraph_id);
    format!("paragraphs/{sanitized}.json")
}

fn sanitize_identifier(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
        } else {
            sanitized.push('-');
        }
    }
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let digest = hasher.finalize();
        digest[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    } else {
        trimmed
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedSlice<'a> {
    pub manifest: SliceManifest,
    pub path: PathBuf,
    pub paragraphs: Vec<&'a ConvertedParagraph>,
    pub cases: Vec<CaseRef<'a>>,
}

#[derive(Debug, Clone)]
pub struct SliceWindow<'a> {
    pub offset: usize,
    pub length: usize,
    pub total_cases: usize,
    pub cases: Vec<CaseRef<'a>>,
    positive_paragraph_ids: Vec<String>,
}

impl<'a> SliceWindow<'a> {
    pub fn positive_ids(&self) -> impl Iterator<Item = &str> {
        self.positive_paragraph_ids.iter().map(|id| id.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct CaseRef<'a> {
    pub paragraph: &'a ConvertedParagraph,
    pub question: &'a ConvertedQuestion,
}

struct DatasetIndex {
    paragraph_by_id: HashMap<String, usize>,
    question_by_id: HashMap<String, (usize, usize)>,
}

impl DatasetIndex {
    fn build(dataset: &ConvertedDataset) -> Self {
        let mut paragraph_by_id = HashMap::new();
        let mut question_by_id = HashMap::new();

        for (p_idx, paragraph) in dataset.paragraphs.iter().enumerate() {
            paragraph_by_id.insert(paragraph.id.clone(), p_idx);
            for (q_idx, question) in paragraph.questions.iter().enumerate() {
                question_by_id.insert(question.id.clone(), (p_idx, q_idx));
            }
        }

        Self {
            paragraph_by_id,
            question_by_id,
        }
    }

    fn paragraph<'a>(
        &self,
        dataset: &'a ConvertedDataset,
        id: &str,
    ) -> Result<&'a ConvertedParagraph> {
        let idx = self
            .paragraph_by_id
            .get(id)
            .ok_or_else(|| anyhow!("slice references unknown paragraph '{id}'"))?;
        Ok(&dataset.paragraphs[*idx])
    }

    fn question<'a>(
        &self,
        dataset: &'a ConvertedDataset,
        question_id: &str,
    ) -> Result<(&'a ConvertedParagraph, &'a ConvertedQuestion)> {
        let (p_idx, q_idx) = self
            .question_by_id
            .get(question_id)
            .ok_or_else(|| anyhow!("slice references unknown question '{question_id}'"))?;
        let paragraph = &dataset.paragraphs[*p_idx];
        let question = paragraph
            .questions
            .get(*q_idx)
            .ok_or_else(|| anyhow!("slice maps question '{question_id}' to missing index"))?;
        Ok((paragraph, question))
    }
}

#[derive(Debug, Serialize)]
struct SliceKey<'a> {
    dataset_id: &'a str,
    includes_unanswerable: bool,
    require_verified_chunks: bool,
    requested_corpus: usize,
    seed: u64,
}

#[derive(Debug)]
struct BuildParams {
    include_impossible: bool,
    base_seed: u64,
    rng_seed: u64,
}

pub fn resolve_slice<'a>(
    dataset: &'a ConvertedDataset,
    config: &SliceConfig<'_>,
) -> Result<ResolvedSlice<'a>> {
    let index = DatasetIndex::build(dataset);

    if let Some(slice_arg) = config.explicit_slice {
        let (path, manifest) = load_explicit_slice(dataset, &index, config, slice_arg)?;
        let resolved = manifest_to_resolved(dataset, &index, manifest, path)?;
        info!(
            slice = %resolved.manifest.slice_id,
            path = %resolved.path.display(),
            cases = resolved.manifest.case_count,
            positives = resolved.manifest.positive_paragraphs,
            negatives = resolved.manifest.negative_paragraphs,
            "Using explicitly selected slice"
        );
        return Ok(resolved);
    }

    let requested_corpus = config
        .corpus_limit
        .unwrap_or(dataset.paragraphs.len())
        .min(dataset.paragraphs.len())
        .max(1);
    let key = SliceKey {
        dataset_id: dataset.metadata.id.as_str(),
        includes_unanswerable: config.llm_mode,
        require_verified_chunks: config.require_verified_chunks,
        requested_corpus,
        seed: config.slice_seed,
    };
    let slice_id = compute_slice_id(&key);
    let base = config
        .cache_dir
        .join("slices")
        .join(dataset.metadata.id.as_str());
    let path = base.join(format!("{slice_id}.json"));

    let total_questions = dataset
        .paragraphs
        .iter()
        .map(|p| p.questions.len())
        .sum::<usize>()
        .max(1);
    let requested_limit = config
        .limit
        .unwrap_or(total_questions)
        .min(total_questions)
        .max(1);

    let mut manifest = if !config.force_convert && path.exists() {
        match read_manifest(&path) {
            Ok(manifest) if manifest.dataset_id == dataset.metadata.id => {
                if manifest.includes_unanswerable != config.llm_mode {
                    warn!(
                        slice = manifest.slice_id,
                        path = %path.display(),
                        expected = config.llm_mode,
                        found = manifest.includes_unanswerable,
                        "Slice manifest includes_unanswerable mismatch; regenerating"
                    );
                    None
                } else if manifest.require_verified_chunks != config.require_verified_chunks {
                    warn!(
                        slice = manifest.slice_id,
                        path = %path.display(),
                        expected = config.require_verified_chunks,
                        found = manifest.require_verified_chunks,
                        "Slice manifest verified-chunk requirement mismatch; regenerating"
                    );
                    None
                } else {
                    Some(manifest)
                }
            }
            Ok(manifest) => {
                warn!(
                    slice = manifest.slice_id,
                    path = %path.display(),
                    loaded_dataset = %manifest.dataset_id,
                    expected = %dataset.metadata.id,
                    "Slice manifest targets different dataset; regenerating"
                );
                None
            }
            Err(err) => {
                warn!(
                    path = %path.display(),
                    error = %err,
                    "Failed to read cached slice; regenerating"
                );
                None
            }
        }
    } else {
        None
    };

    let params = BuildParams {
        include_impossible: config.llm_mode,
        base_seed: config.slice_seed,
        rng_seed: mix_seed(dataset.metadata.id.as_str(), config.slice_seed),
    };

    if manifest
        .as_ref()
        .map(|manifest| manifest.version != SLICE_VERSION)
        .unwrap_or(false)
    {
        warn!(
            slice = manifest
                .as_ref()
                .map(|m| m.slice_id.as_str())
                .unwrap_or("unknown"),
            found = manifest.as_ref().map(|m| m.version).unwrap_or(0),
            expected = SLICE_VERSION,
            "Slice manifest version mismatch; regenerating"
        );
        manifest = None;
    }

    let mut manifest = manifest.unwrap_or_else(|| {
        empty_manifest(
            dataset,
            slice_id.clone(),
            &params,
            requested_corpus,
            config.negative_multiplier,
            config.require_verified_chunks,
            config.limit,
        )
    });

    manifest.requested_limit = config.limit;
    manifest.requested_corpus = requested_corpus;
    manifest.negative_multiplier = config.negative_multiplier;
    manifest.includes_unanswerable = config.llm_mode;
    manifest.require_verified_chunks = config.require_verified_chunks;

    let mut changed = ensure_shard_paths(&mut manifest);

    changed |= ensure_case_capacity(dataset, &mut manifest, &params, requested_limit)?;
    refresh_manifest_stats(&mut manifest);

    let desired_negatives = desired_negative_target(
        manifest.positive_paragraphs,
        requested_corpus,
        dataset.paragraphs.len(),
        config.negative_multiplier,
    );
    changed |= ensure_negative_pool(
        dataset,
        &mut manifest,
        &params,
        desired_negatives,
        requested_corpus,
    )?;
    refresh_manifest_stats(&mut manifest);

    if changed {
        manifest.generated_at = Utc::now();
        write_manifest(&path, &manifest)?;
        info!(
            slice = %manifest.slice_id,
            path = %path.display(),
            cases = manifest.case_count,
            positives = manifest.positive_paragraphs,
            negatives = manifest.negative_paragraphs,
            "Updated dataset slice ledger"
        );
    } else {
        info!(
            slice = %manifest.slice_id,
            path = %path.display(),
            cases = manifest.case_count,
            positives = manifest.positive_paragraphs,
            negatives = manifest.negative_paragraphs,
            "Reusing cached slice ledger"
        );
    }

    let resolved = manifest_to_resolved(dataset, &index, manifest.clone(), path.clone())?;

    Ok(resolved)
}

pub fn select_window<'a>(
    resolved: &'a ResolvedSlice<'a>,
    offset: usize,
    limit: Option<usize>,
) -> Result<SliceWindow<'a>> {
    let total = resolved.manifest.case_count;
    if total == 0 {
        return Err(anyhow!(
            "slice '{}' contains no cases",
            resolved.manifest.slice_id
        ));
    }
    if offset >= total {
        return Err(anyhow!(
            "slice offset {} exceeds available cases ({})",
            offset,
            total
        ));
    }
    let available = total - offset;
    let requested = limit.unwrap_or(available).max(1);
    let length = requested.min(available);
    let cases = resolved.cases[offset..offset + length].to_vec();
    let mut seen = HashSet::new();
    let mut positive_ids = Vec::new();
    for case in &cases {
        if seen.insert(case.paragraph.id.as_str()) {
            positive_ids.push(case.paragraph.id.clone());
        }
    }
    Ok(SliceWindow {
        offset,
        length,
        total_cases: total,
        cases,
        positive_paragraph_ids: positive_ids,
    })
}

#[allow(dead_code)]
pub fn full_window<'a>(resolved: &'a ResolvedSlice<'a>) -> Result<SliceWindow<'a>> {
    select_window(resolved, 0, None)
}

fn load_explicit_slice<'a>(
    dataset: &'a ConvertedDataset,
    index: &DatasetIndex,
    config: &SliceConfig<'_>,
    slice_arg: &str,
) -> Result<(PathBuf, SliceManifest)> {
    let explicit_path = Path::new(slice_arg);
    let candidate_path = if explicit_path.exists() {
        explicit_path.to_path_buf()
    } else {
        config
            .cache_dir
            .join("slices")
            .join(dataset.metadata.id.as_str())
            .join(format!("{slice_arg}.json"))
    };

    let manifest = read_manifest(&candidate_path)
        .with_context(|| format!("reading slice manifest at {}", candidate_path.display()))?;

    if manifest.dataset_id != dataset.metadata.id {
        return Err(anyhow!(
            "slice '{}' targets dataset '{}', but '{}' is loaded",
            manifest.slice_id,
            manifest.dataset_id,
            dataset.metadata.id
        ));
    }
    if manifest.includes_unanswerable != config.llm_mode {
        return Err(anyhow!(
            "slice '{}' includes_unanswerable mismatch (expected {}, found {})",
            manifest.slice_id,
            config.llm_mode,
            manifest.includes_unanswerable
        ));
    }
    if manifest.require_verified_chunks != config.require_verified_chunks {
        return Err(anyhow!(
            "slice '{}' verified-chunk requirement mismatch (expected {}, found {})",
            manifest.slice_id,
            config.require_verified_chunks,
            manifest.require_verified_chunks
        ));
    }

    // Validate the manifest before returning.
    manifest_to_resolved(dataset, index, manifest.clone(), candidate_path.clone())?;

    Ok((candidate_path, manifest))
}

fn empty_manifest(
    dataset: &ConvertedDataset,
    slice_id: String,
    params: &BuildParams,
    requested_corpus: usize,
    negative_multiplier: f32,
    require_verified_chunks: bool,
    requested_limit: Option<usize>,
) -> SliceManifest {
    SliceManifest {
        version: SLICE_VERSION,
        slice_id,
        dataset_id: dataset.metadata.id.clone(),
        dataset_label: dataset.metadata.label.clone(),
        dataset_source: dataset.source.clone(),
        includes_unanswerable: params.include_impossible,
        require_verified_chunks,
        seed: params.base_seed,
        requested_limit,
        requested_corpus,
        negative_multiplier,
        generated_at: Utc::now(),
        case_count: 0,
        positive_paragraphs: 0,
        negative_paragraphs: 0,
        total_paragraphs: 0,
        cases: Vec::new(),
        paragraphs: Vec::new(),
    }
}

fn ensure_case_capacity(
    dataset: &ConvertedDataset,
    manifest: &mut SliceManifest,
    params: &BuildParams,
    target_cases: usize,
) -> Result<bool> {
    if manifest.case_count >= target_cases {
        return Ok(false);
    }

    let question_refs = ordered_question_refs(dataset, params, target_cases)?;
    let mut existing_questions: HashSet<String> = manifest
        .cases
        .iter()
        .map(|case| case.question_id.clone())
        .collect();
    let mut paragraph_positions: HashMap<String, usize> = manifest
        .paragraphs
        .iter()
        .enumerate()
        .map(|(idx, entry)| (entry.id.clone(), idx))
        .collect();

    let mut changed = false;

    for (p_idx, q_idx) in question_refs {
        if manifest.case_count >= target_cases {
            break;
        }
        let paragraph = &dataset.paragraphs[p_idx];
        let question = &paragraph.questions[q_idx];
        if !existing_questions.insert(question.id.clone()) {
            continue;
        }

        if let Some(idx) = paragraph_positions.get(paragraph.id.as_str()).copied() {
            match &mut manifest.paragraphs[idx].kind {
                SliceParagraphKind::Positive { question_ids } => {
                    if !question_ids.contains(&question.id) {
                        question_ids.push(question.id.clone());
                    }
                }
                SliceParagraphKind::Negative => {
                    manifest.paragraphs[idx].kind = SliceParagraphKind::Positive {
                        question_ids: vec![question.id.clone()],
                    };
                }
            }
        } else {
            manifest.paragraphs.push(SliceParagraphEntry {
                id: paragraph.id.clone(),
                kind: SliceParagraphKind::Positive {
                    question_ids: vec![question.id.clone()],
                },
                shard_path: Some(default_shard_path(&paragraph.id)),
            });
            let idx = manifest.paragraphs.len() - 1;
            paragraph_positions.insert(paragraph.id.clone(), idx);
        }

        manifest.cases.push(SliceCaseEntry {
            question_id: question.id.clone(),
            paragraph_id: paragraph.id.clone(),
        });
        manifest.case_count += 1;
        changed = true;
    }

    if manifest.case_count < target_cases {
        return Err(anyhow!(
            "only {}/{} eligible questions available for dataset {}",
            manifest.case_count,
            target_cases,
            dataset.metadata.id
        ));
    }

    Ok(changed)
}

fn ordered_question_refs(
    dataset: &ConvertedDataset,
    params: &BuildParams,
    target_cases: usize,
) -> Result<Vec<(usize, usize)>> {
    if dataset.metadata.id == DatasetKind::Beir.id() {
        return ordered_question_refs_beir(dataset, params, target_cases);
    }

    let mut question_refs = Vec::new();
    for (p_idx, paragraph) in dataset.paragraphs.iter().enumerate() {
        for (q_idx, question) in paragraph.questions.iter().enumerate() {
            let include = if params.include_impossible {
                true
            } else {
                !question.is_impossible && !question.answers.is_empty()
            };
            if include {
                question_refs.push((p_idx, q_idx));
            }
        }
    }

    if question_refs.is_empty() {
        return Err(anyhow!(
            "no eligible questions found for dataset {}; cannot build slice",
            dataset.metadata.id
        ));
    }

    let mut rng = StdRng::seed_from_u64(params.rng_seed);
    question_refs.shuffle(&mut rng);
    Ok(question_refs)
}

fn ordered_question_refs_beir(
    dataset: &ConvertedDataset,
    params: &BuildParams,
    target_cases: usize,
) -> Result<Vec<(usize, usize)>> {
    let prefixes: Vec<&str> = BEIR_DATASETS
        .iter()
        .map(|kind| kind.source_prefix())
        .collect();

    let mut grouped: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for (p_idx, paragraph) in dataset.paragraphs.iter().enumerate() {
        for (q_idx, question) in paragraph.questions.iter().enumerate() {
            let include = if params.include_impossible {
                true
            } else {
                !question.is_impossible && !question.answers.is_empty()
            };
            if !include {
                continue;
            }

            let Some(prefix) = question_prefix(&question.id) else {
                warn!(
                    question_id = %question.id,
                    "Skipping BEIR question without expected prefix"
                );
                continue;
            };
            if !prefixes.contains(&prefix) {
                warn!(
                    question_id = %question.id,
                    prefix = %prefix,
                    "Skipping BEIR question with unknown subset prefix"
                );
                continue;
            }
            grouped.entry(prefix).or_default().push((p_idx, q_idx));
        }
    }

    if grouped.values().all(|entries| entries.is_empty()) {
        return Err(anyhow!(
            "no eligible BEIR questions found; cannot build slice"
        ));
    }

    for prefix in &prefixes {
        if let Some(entries) = grouped.get_mut(prefix) {
            let seed = mix_seed(
                &format!("{}::{prefix}", dataset.metadata.id),
                params.base_seed,
            );
            let mut rng = StdRng::seed_from_u64(seed);
            entries.shuffle(&mut rng);
        }
    }

    let dataset_count = prefixes.len().max(1);
    let base_quota = target_cases / dataset_count;
    let mut remainder = target_cases % dataset_count;

    let mut quotas: HashMap<&str, usize> = HashMap::new();
    for prefix in &prefixes {
        let mut quota = base_quota;
        if remainder > 0 {
            quota += 1;
            remainder -= 1;
        }
        quotas.insert(*prefix, quota);
    }

    let mut take_counts: HashMap<&str, usize> = HashMap::new();
    let mut spare_slots: HashMap<&str, usize> = HashMap::new();
    let mut shortfall = 0usize;

    for prefix in &prefixes {
        let available = grouped.get(prefix).map(|v| v.len()).unwrap_or(0);
        let quota = *quotas.get(prefix).unwrap_or(&0);
        let take = quota.min(available);
        let missing = quota.saturating_sub(take);
        shortfall += missing;
        take_counts.insert(*prefix, take);
        spare_slots.insert(*prefix, available.saturating_sub(take));
    }

    while shortfall > 0 {
        let mut allocated = false;
        for prefix in &prefixes {
            if shortfall == 0 {
                break;
            }
            let spare = spare_slots.get(prefix).copied().unwrap_or(0);
            if spare == 0 {
                continue;
            }
            if let Some(count) = take_counts.get_mut(prefix) {
                *count += 1;
            }
            spare_slots.insert(*prefix, spare - 1);
            shortfall = shortfall.saturating_sub(1);
            allocated = true;
        }
        if !allocated {
            break;
        }
    }

    let mut queues: Vec<VecDeque<(usize, usize)>> = Vec::new();
    let mut total_selected = 0usize;
    for prefix in &prefixes {
        let take = *take_counts.get(prefix).unwrap_or(&0);
        let mut deque = VecDeque::new();
        if let Some(entries) = grouped.get(prefix) {
            for item in entries.iter().take(take) {
                deque.push_back(*item);
                total_selected += 1;
            }
        }
        queues.push(deque);
    }

    if total_selected < target_cases {
        warn!(
            requested = target_cases,
            available = total_selected,
            "BEIR mix requested more questions than available after balancing; continuing with capped set"
        );
    }

    let mut output = Vec::with_capacity(total_selected);
    loop {
        let mut progressed = false;
        for queue in queues.iter_mut() {
            if let Some(item) = queue.pop_front() {
                output.push(item);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }

    if output.is_empty() {
        return Err(anyhow!(
            "no eligible BEIR questions found; cannot build slice"
        ));
    }

    Ok(output)
}

fn question_prefix(question_id: &str) -> Option<&'static str> {
    for prefix in BEIR_DATASETS.iter().map(|kind| kind.source_prefix()) {
        if let Some(rest) = question_id.strip_prefix(prefix) {
            if rest.starts_with('-') {
                return Some(prefix);
            }
        }
    }
    None
}

fn ensure_negative_pool(
    dataset: &ConvertedDataset,
    manifest: &mut SliceManifest,
    params: &BuildParams,
    target_negatives: usize,
    requested_corpus: usize,
) -> Result<bool> {
    let current_negatives = manifest
        .paragraphs
        .iter()
        .filter(|entry| matches!(entry.kind, SliceParagraphKind::Negative))
        .count();
    if current_negatives >= target_negatives {
        return Ok(false);
    }

    let positive_ids: HashSet<String> = manifest
        .paragraphs
        .iter()
        .filter_map(|entry| match entry.kind {
            SliceParagraphKind::Positive { .. } => Some(entry.id.clone()),
            _ => None,
        })
        .collect();
    let mut negative_ids: HashSet<String> = manifest
        .paragraphs
        .iter()
        .filter_map(|entry| match entry.kind {
            SliceParagraphKind::Negative => Some(entry.id.clone()),
            _ => None,
        })
        .collect();

    let negative_seed = mix_seed(
        &format!("{}::negatives", dataset.metadata.id),
        params.base_seed,
    );
    let candidates = ordered_negative_indices(dataset, &positive_ids, negative_seed);
    let mut added = false;
    for idx in candidates {
        if negative_ids.len() >= target_negatives {
            break;
        }
        let paragraph = &dataset.paragraphs[idx];
        if negative_ids.contains(paragraph.id.as_str())
            || positive_ids.contains(paragraph.id.as_str())
        {
            continue;
        }
        manifest.paragraphs.push(SliceParagraphEntry {
            id: paragraph.id.clone(),
            kind: SliceParagraphKind::Negative,
            shard_path: Some(default_shard_path(&paragraph.id)),
        });
        negative_ids.insert(paragraph.id.clone());
        added = true;
    }

    if negative_ids.len() < target_negatives {
        warn!(
            dataset = %dataset.metadata.id,
            desired = target_negatives,
            available = negative_ids.len(),
            requested_corpus,
            "Insufficient negative paragraphs to satisfy multiplier"
        );
    }

    Ok(added)
}

fn ordered_negative_indices(
    dataset: &ConvertedDataset,
    positive_ids: &HashSet<String>,
    rng_seed: u64,
) -> Vec<usize> {
    let mut candidates: Vec<usize> = dataset
        .paragraphs
        .iter()
        .enumerate()
        .filter_map(|(idx, paragraph)| {
            if positive_ids.contains(paragraph.id.as_str()) {
                None
            } else {
                Some(idx)
            }
        })
        .collect();
    let mut rng = StdRng::seed_from_u64(rng_seed);
    candidates.shuffle(&mut rng);
    candidates
}

fn refresh_manifest_stats(manifest: &mut SliceManifest) {
    manifest.case_count = manifest.cases.len();
    manifest.positive_paragraphs = manifest
        .paragraphs
        .iter()
        .filter(|entry| matches!(entry.kind, SliceParagraphKind::Positive { .. }))
        .count();
    manifest.negative_paragraphs = manifest
        .paragraphs
        .iter()
        .filter(|entry| matches!(entry.kind, SliceParagraphKind::Negative))
        .count();
    manifest.total_paragraphs = manifest.paragraphs.len();
}

fn ensure_shard_paths(manifest: &mut SliceManifest) -> bool {
    let mut changed = false;
    for entry in &mut manifest.paragraphs {
        if entry.shard_path.is_none() {
            entry.shard_path = Some(default_shard_path(&entry.id));
            changed = true;
        }
    }
    changed
}

fn desired_negative_target(
    positive_count: usize,
    requested_corpus: usize,
    dataset_paragraphs: usize,
    multiplier: f32,
) -> usize {
    if positive_count == 0 {
        return 0;
    }
    let ratio = multiplier.max(0.0);
    let mut desired = ((positive_count as f32) * ratio).ceil() as usize;
    let max_total = requested_corpus.min(dataset_paragraphs).max(positive_count);
    let max_negatives = max_total.saturating_sub(positive_count);
    desired = desired.min(max_negatives);
    desired
}

fn manifest_to_resolved<'a>(
    dataset: &'a ConvertedDataset,
    index: &DatasetIndex,
    manifest: SliceManifest,
    path: PathBuf,
) -> Result<ResolvedSlice<'a>> {
    if manifest.version != SLICE_VERSION {
        return Err(anyhow!(
            "slice version {} does not match expected {}",
            manifest.version,
            SLICE_VERSION
        ));
    }

    let mut paragraphs = Vec::with_capacity(manifest.paragraphs.len());
    for entry in &manifest.paragraphs {
        let paragraph = index.paragraph(dataset, &entry.id)?;
        if let SliceParagraphKind::Positive { question_ids } = &entry.kind {
            for question_id in question_ids {
                let (linked_paragraph, _) = index.question(dataset, question_id)?;
                if linked_paragraph.id != entry.id {
                    return Err(anyhow!(
                        "slice question '{}' expected paragraph '{}', found '{}'",
                        question_id,
                        entry.id,
                        linked_paragraph.id
                    ));
                }
            }
        }
        paragraphs.push(paragraph);
    }

    let mut cases = Vec::with_capacity(manifest.cases.len());
    for entry in &manifest.cases {
        let (paragraph, question) = index.question(dataset, &entry.question_id)?;
        if paragraph.id != entry.paragraph_id {
            return Err(anyhow!(
                "slice case '{}' expected paragraph '{}', found '{}'",
                entry.question_id,
                entry.paragraph_id,
                paragraph.id
            ));
        }
        cases.push(CaseRef {
            paragraph,
            question,
        });
    }

    if cases.is_empty() {
        return Err(anyhow!(
            "slice '{}' contains no cases after validation",
            manifest.slice_id
        ));
    }

    Ok(ResolvedSlice {
        manifest,
        path,
        paragraphs,
        cases,
    })
}

fn compute_slice_id(key: &SliceKey<'_>) -> String {
    let payload = serde_json::to_vec(key).expect("SliceKey serialisation should not fail");
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn mix_seed(dataset_id: &str, seed: u64) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(dataset_id.as_bytes());
    hasher.update(seed.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes)
}

fn read_manifest(path: &Path) -> Result<SliceManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading slice manifest {}", path.display()))?;
    let manifest: SliceManifest = serde_json::from_str(&raw)
        .with_context(|| format!("parsing slice manifest {}", path.display()))?;
    Ok(manifest)
}

fn write_manifest(path: &Path, manifest: &SliceManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating slice directory {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(manifest).context("serialising slice manifest to JSON")?;
    fs::write(path, json).with_context(|| format!("writing slice manifest {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::{
        ConvertedDataset, ConvertedParagraph, ConvertedQuestion, DatasetKind, DatasetMetadata,
    };
    use tempfile::tempdir;

    fn sample_dataset() -> ConvertedDataset {
        let metadata = DatasetMetadata::for_kind(DatasetKind::SquadV2, false, None);
        ConvertedDataset {
            generated_at: Utc::now(),
            metadata,
            source: "test-source".to_string(),
            paragraphs: vec![
                ConvertedParagraph {
                    id: "p1".to_string(),
                    title: "Alpha".to_string(),
                    context: "Alpha context".to_string(),
                    questions: vec![ConvertedQuestion {
                        id: "q1".to_string(),
                        question: "What is alpha?".to_string(),
                        answers: vec!["Alpha".to_string()],
                        is_impossible: false,
                    }],
                },
                ConvertedParagraph {
                    id: "p2".to_string(),
                    title: "Beta".to_string(),
                    context: "Beta context".to_string(),
                    questions: vec![ConvertedQuestion {
                        id: "q2".to_string(),
                        question: "What is beta?".to_string(),
                        answers: vec!["Beta".to_string()],
                        is_impossible: false,
                    }],
                },
                ConvertedParagraph {
                    id: "p3".to_string(),
                    title: "Gamma".to_string(),
                    context: "Gamma context".to_string(),
                    questions: vec![ConvertedQuestion {
                        id: "q3".to_string(),
                        question: "What is gamma?".to_string(),
                        answers: vec!["Gamma".to_string()],
                        is_impossible: false,
                    }],
                },
            ],
        }
    }

    #[test]
    fn resolve_slice_reuses_cached_manifest() -> Result<()> {
        let dataset = sample_dataset();
        let temp = tempdir().context("creating temp directory")?;

        let mut config = SliceConfig {
            cache_dir: temp.path(),
            force_convert: false,
            explicit_slice: None,
            limit: Some(2),
            corpus_limit: Some(3),
            slice_seed: 0x5eed_2025,
            llm_mode: false,
            negative_multiplier: DEFAULT_NEGATIVE_MULTIPLIER,
            require_verified_chunks: true,
        };

        let first = resolve_slice(&dataset, &config)?;
        assert!(first.path.exists());
        let initial_generated = first.manifest.generated_at;

        let second = resolve_slice(&dataset, &config)?;
        assert_eq!(first.manifest.slice_id, second.manifest.slice_id);
        assert_eq!(initial_generated, second.manifest.generated_at);

        config.force_convert = true;
        let third = resolve_slice(&dataset, &config)?;
        assert_eq!(first.manifest.slice_id, third.manifest.slice_id);
        assert_ne!(third.manifest.generated_at, initial_generated);

        Ok(())
    }

    #[test]
    fn select_window_yields_expected_cases() -> Result<()> {
        let dataset = sample_dataset();
        let temp = tempdir().context("creating temp directory")?;
        let config = SliceConfig {
            cache_dir: temp.path(),
            force_convert: false,
            explicit_slice: None,
            limit: Some(3),
            corpus_limit: Some(3),
            slice_seed: 0x5eed_2025,
            llm_mode: false,
            negative_multiplier: DEFAULT_NEGATIVE_MULTIPLIER,
            require_verified_chunks: true,
        };
        let resolved = resolve_slice(&dataset, &config)?;
        let window = select_window(&resolved, 1, Some(1))?;
        assert_eq!(window.offset, 1);
        assert_eq!(window.length, 1);
        assert_eq!(window.total_cases, resolved.manifest.case_count);
        assert_eq!(window.cases.len(), 1);
        let positive_ids: Vec<&str> = window.positive_ids().collect();
        assert_eq!(positive_ids.len(), 1);
        assert!(resolved
            .manifest
            .paragraphs
            .iter()
            .any(|entry| entry.id == positive_ids[0]));
        Ok(())
    }

    #[test]
    fn beir_mix_balances_and_rebalances() -> Result<()> {
        let mut paragraphs = Vec::new();
        let counts = [
            ("fever", 1usize),
            ("fiqa", 2usize),
            ("hotpotqa", 1usize),
            ("nfcorpus", 0usize),
            ("quora", 3usize),
            ("trec-covid", 2usize),
        ];

        for (prefix, count) in counts {
            for idx in 0..count {
                let q_id = format!("{prefix}-q{idx}");
                paragraphs.push(ConvertedParagraph {
                    id: format!("{prefix}-p{idx}"),
                    title: format!("{prefix} title"),
                    context: format!("{prefix} context {idx}"),
                    questions: vec![ConvertedQuestion {
                        id: q_id,
                        question: format!("{prefix} question {idx}"),
                        answers: vec!["answer".to_string()],
                        is_impossible: false,
                    }],
                });
            }
        }

        let metadata = DatasetMetadata::for_kind(DatasetKind::Beir, false, None);
        let dataset = ConvertedDataset {
            generated_at: Utc::now(),
            metadata,
            source: "beir-mix".to_string(),
            paragraphs,
        };

        let params = BuildParams {
            include_impossible: false,
            base_seed: 0xAA,
            rng_seed: 0xBB,
        };

        let refs = ordered_question_refs_beir(&dataset, &params, 8)?;
        let mut per_prefix: HashMap<String, usize> = HashMap::new();
        for (p_idx, q_idx) in refs {
            let question = &dataset.paragraphs[p_idx].questions[q_idx];
            let prefix = question_prefix(&question.id).unwrap_or("unknown");
            *per_prefix.entry(prefix.to_string()).or_default() += 1;
        }

        assert_eq!(per_prefix.get("fever").copied().unwrap_or(0), 1);
        assert_eq!(per_prefix.get("fiqa").copied().unwrap_or(0), 2);
        assert_eq!(per_prefix.get("hotpotqa").copied().unwrap_or(0), 1);
        assert_eq!(per_prefix.get("nfcorpus").copied().unwrap_or(0), 0);
        assert_eq!(per_prefix.get("quora").copied().unwrap_or(0), 2);
        assert_eq!(per_prefix.get("trec-covid").copied().unwrap_or(0), 2);

        Ok(())
    }
}

// MARK: - Config integration (merged from slice.rs)

use crate::args::Config;

impl<'a> From<&'a Config> for SliceConfig<'a> {
    fn from(config: &'a Config) -> Self {
        slice_config_with_limit(config, None)
    }
}

pub fn slice_config_with_limit<'a>(
    config: &'a Config,
    limit_override: Option<usize>,
) -> SliceConfig<'a> {
    SliceConfig {
        cache_dir: config.cache_dir.as_path(),
        force_convert: config.force_convert,
        explicit_slice: config.slice.as_deref(),
        limit: limit_override.or(config.limit),
        corpus_limit: config.corpus_limit,
        slice_seed: config.slice_seed,
        llm_mode: config.llm_mode,
        negative_multiplier: config.negative_multiplier,
        require_verified_chunks: config.retrieval.require_verified_chunks,
    }
}
