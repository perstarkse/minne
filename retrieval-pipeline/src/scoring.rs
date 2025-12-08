use std::{cmp::Ordering, collections::HashMap};

use common::storage::types::StoredObject;
use serde::{Deserialize, Serialize};

/// Holds optional subscores gathered from different retrieval signals.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scores {
    pub fts: Option<f32>,
    pub vector: Option<f32>,
    pub graph: Option<f32>,
}

/// Generic wrapper combining an item with its accumulated retrieval scores.
#[derive(Debug, Clone)]
pub struct Scored<T> {
    pub item: T,
    pub scores: Scores,
    pub fused: f32,
}

impl<T> Scored<T> {
    pub fn new(item: T) -> Self {
        Self {
            item,
            scores: Scores::default(),
            fused: 0.0,
        }
    }

    pub const fn with_vector_score(mut self, score: f32) -> Self {
        self.scores.vector = Some(score);
        self
    }

    pub const fn with_fts_score(mut self, score: f32) -> Self {
        self.scores.fts = Some(score);
        self
    }

    pub const fn with_graph_score(mut self, score: f32) -> Self {
        self.scores.graph = Some(score);
        self
    }

    pub const fn update_fused(&mut self, fused: f32) {
        self.fused = fused;
    }
}

/// Weights used for linear score fusion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FusionWeights {
    pub vector: f32,
    pub fts: f32,
    pub graph: f32,
    pub multi_bonus: f32,
}

impl Default for FusionWeights {
    fn default() -> Self {
        // Default weights favor vector search, which typically performs better
        // FTS is used as a complement when there's good overlap
        // Higher multi_bonus to heavily favor chunks with both signals (the "golden chunk")
        Self {
            vector: 0.8,
            fts: 0.2,
            graph: 0.2,
            multi_bonus: 0.3, // Increased to boost chunks with both signals
        }
    }
}

/// Configuration for reciprocal rank fusion.
#[derive(Debug, Clone, Copy)]
pub struct RrfConfig {
    pub k: f32,
    pub vector_weight: f32,
    pub fts_weight: f32,
    pub use_vector: bool,
    pub use_fts: bool,
}

impl Default for RrfConfig {
    fn default() -> Self {
        Self {
            k: 60.0,
            vector_weight: 1.0,
            fts_weight: 1.0,
            use_vector: true,
            use_fts: true,
        }
    }
}

pub const fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

pub fn distance_to_similarity(distance: f32) -> f32 {
    if !distance.is_finite() {
        return 0.0;
    }
    clamp_unit(1.0 / (1.0 + distance.max(0.0)))
}

pub fn min_max_normalize(scores: &[f32]) -> Vec<f32> {
    if scores.is_empty() {
        return Vec::new();
    }

    let mut min = f32::MAX;
    let mut max = f32::MIN;

    for s in scores {
        if !s.is_finite() {
            continue;
        }
        if *s < min {
            min = *s;
        }
        if *s > max {
            max = *s;
        }
    }

    if !min.is_finite() || !max.is_finite() {
        return scores.iter().map(|_| 0.0).collect();
    }

    if (max - min).abs() < f32::EPSILON {
        return vec![1.0; scores.len()];
    }

    scores
        .iter()
        .map(|score| {
            if score.is_finite() {
                clamp_unit((score - min) / (max - min))
            } else {
                0.0
            }
        })
        .collect()
}

pub fn fuse_scores(scores: &Scores, weights: FusionWeights) -> f32 {
    let vector = scores.vector.unwrap_or(0.0);
    let fts = scores.fts.unwrap_or(0.0);
    let graph = scores.graph.unwrap_or(0.0);

    let mut fused = graph.mul_add(
        weights.graph,
        vector.mul_add(weights.vector, fts * weights.fts),
    );

    let signals_present = scores
        .vector
        .iter()
        .chain(scores.fts.iter())
        .chain(scores.graph.iter())
        .count();

    // Boost chunks with multiple signals (especially vector + FTS, the "golden chunk")
    if signals_present >= 2 {
        // For chunks with both vector and FTS, give a significant boost
        // This helps identify the "golden chunk" that appears in both searches
        if scores.vector.is_some() && scores.fts.is_some() {
            // Multiplicative boost: multiply by (1 + bonus) to scale with the base score
            // This ensures high-scoring golden chunks get boosted more than low-scoring ones
            fused = fused * (1.0 + weights.multi_bonus);
        } else {
            // For other multi-signal combinations (e.g., vector + graph), use additive bonus
            fused += weights.multi_bonus;
        }
    }

    clamp_unit(fused)
}

pub fn merge_scored_by_id<T>(
    target: &mut std::collections::HashMap<String, Scored<T>>,
    incoming: Vec<Scored<T>>,
) where
    T: StoredObject + Clone,
{
    for scored in incoming {
        let id = scored.item.get_id().to_owned();
        target
            .entry(id)
            .and_modify(|existing| {
                if let Some(score) = scored.scores.vector {
                    existing.scores.vector = Some(score);
                }
                if let Some(score) = scored.scores.fts {
                    existing.scores.fts = Some(score);
                }
                if let Some(score) = scored.scores.graph {
                    existing.scores.graph = Some(score);
                }
            })
            .or_insert_with(|| Scored {
                item: scored.item.clone(),
                scores: scored.scores,
                fused: scored.fused,
            });
    }
}

pub fn sort_by_fused_desc<T>(items: &mut [Scored<T>])
where
    T: StoredObject,
{
    items.sort_by(|a, b| {
        b.fused
            .partial_cmp(&a.fused)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.item.get_id().cmp(b.item.get_id()))
    });
}

pub fn reciprocal_rank_fusion<T>(
    mut vector_ranked: Vec<Scored<T>>,
    mut fts_ranked: Vec<Scored<T>>,
    config: RrfConfig,
) -> Vec<Scored<T>>
where
    T: StoredObject + Clone,
{
    let mut merged: HashMap<String, Scored<T>> = HashMap::new();
    let k = if config.k <= 0.0 { 60.0 } else { config.k };
    let vector_weight = if config.vector_weight.is_finite() {
        config.vector_weight.max(0.0)
    } else {
        0.0
    };
    let fts_weight = if config.fts_weight.is_finite() {
        config.fts_weight.max(0.0)
    } else {
        0.0
    };

    if config.use_vector && !vector_ranked.is_empty() {
        vector_ranked.sort_by(|a, b| {
            let a_score = a.scores.vector.unwrap_or(0.0);
            let b_score = b.scores.vector.unwrap_or(0.0);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.item.get_id().cmp(b.item.get_id()))
        });

        for (rank, candidate) in vector_ranked.into_iter().enumerate() {
            let id = candidate.item.get_id().to_owned();
            let entry = merged
                .entry(id.clone())
                .or_insert_with(|| Scored::new(candidate.item.clone()));

            if let Some(score) = candidate.scores.vector {
                let existing = entry.scores.vector.unwrap_or(f32::MIN);
                if score > existing {
                    entry.scores.vector = Some(score);
                }
            }
            entry.item = candidate.item;
            entry.fused += vector_weight / (k + rank as f32 + 1.0);
        }
    }

    if config.use_fts && !fts_ranked.is_empty() {
        fts_ranked.sort_by(|a, b| {
            let a_score = a.scores.fts.unwrap_or(0.0);
            let b_score = b.scores.fts.unwrap_or(0.0);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.item.get_id().cmp(b.item.get_id()))
        });

        for (rank, candidate) in fts_ranked.into_iter().enumerate() {
            let id = candidate.item.get_id().to_owned();
            let entry = merged
                .entry(id.clone())
                .or_insert_with(|| Scored::new(candidate.item.clone()));

            if let Some(score) = candidate.scores.fts {
                let existing = entry.scores.fts.unwrap_or(f32::MIN);
                if score > existing {
                    entry.scores.fts = Some(score);
                }
            }
            entry.item = candidate.item;
            entry.fused += fts_weight / (k + rank as f32 + 1.0);
        }
    }

    let mut fused: Vec<Scored<T>> = merged.into_values().collect();
    sort_by_fused_desc(&mut fused);
    fused
}
