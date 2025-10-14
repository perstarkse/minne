use std::cmp::Ordering;

use common::storage::types::StoredObject;

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

    pub fn with_vector_score(mut self, score: f32) -> Self {
        self.scores.vector = Some(score);
        self
    }

    pub fn with_fts_score(mut self, score: f32) -> Self {
        self.scores.fts = Some(score);
        self
    }

    pub fn with_graph_score(mut self, score: f32) -> Self {
        self.scores.graph = Some(score);
        self
    }

    pub fn update_fused(&mut self, fused: f32) {
        self.fused = fused;
    }
}

/// Weights used for linear score fusion.
#[derive(Debug, Clone, Copy)]
pub struct FusionWeights {
    pub vector: f32,
    pub fts: f32,
    pub graph: f32,
    pub multi_bonus: f32,
}

impl Default for FusionWeights {
    fn default() -> Self {
        Self {
            vector: 0.5,
            fts: 0.3,
            graph: 0.2,
            multi_bonus: 0.02,
        }
    }
}

pub fn clamp_unit(value: f32) -> f32 {
    value.max(0.0).min(1.0)
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
            if !score.is_finite() {
                0.0
            } else {
                clamp_unit((score - min) / (max - min))
            }
        })
        .collect()
}

pub fn fuse_scores(scores: &Scores, weights: FusionWeights) -> f32 {
    let vector = scores.vector.unwrap_or(0.0);
    let fts = scores.fts.unwrap_or(0.0);
    let graph = scores.graph.unwrap_or(0.0);

    let mut fused = vector * weights.vector + fts * weights.fts + graph * weights.graph;

    let signals_present = scores
        .vector
        .iter()
        .chain(scores.fts.iter())
        .chain(scores.graph.iter())
        .count();
    if signals_present >= 2 {
        fused += weights.multi_bonus;
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
