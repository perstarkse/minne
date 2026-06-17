use std::collections::{HashMap, VecDeque};

use anyhow::{anyhow, Result};
use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};
use tracing::warn;

use crate::datasets::{ConvertedDataset, BEIR_DATASETS};

use super::build::{mix_seed, BuildParams};

#[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
pub(super) fn ordered_question_refs_beir(
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

    if grouped.values().all(std::vec::Vec::is_empty) {
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
        let available = grouped.get(prefix).map_or(0, std::vec::Vec::len);
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
        for queue in &mut queues {
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

pub(super) fn question_prefix(question_id: &str) -> Option<&'static str> {
    for prefix in BEIR_DATASETS.iter().map(|kind| kind.source_prefix()) {
        if let Some(rest) = question_id.strip_prefix(prefix) {
            if rest.starts_with('-') {
                return Some(prefix);
            }
        }
    }
    None
}
