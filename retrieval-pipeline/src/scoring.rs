use std::{
    cmp::Ordering,
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use common::storage::types::{
    knowledge_entity::KnowledgeEntity, text_chunk::TextChunk, StoredObject,
};

/// Identifier access for retrieval fusion and sorting.
pub trait RetrievalCandidate {
    fn candidate_id(&self) -> &str;
}

impl RetrievalCandidate for TextChunk {
    fn candidate_id(&self) -> &str {
        self.id()
    }
}

impl RetrievalCandidate for Arc<TextChunk> {
    fn candidate_id(&self) -> &str {
        self.as_ref().id()
    }
}

impl RetrievalCandidate for KnowledgeEntity {
    fn candidate_id(&self) -> &str {
        self.id()
    }
}

/// Holds optional subscores gathered from the vector and full-text retrieval signals.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scores {
    pub fts: Option<f32>,
    pub vector: Option<f32>,
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

    #[must_use]
    pub const fn with_vector_score(mut self, score: f32) -> Self {
        self.scores.vector = Some(score);
        self
    }

    #[must_use]
    pub const fn with_fts_score(mut self, score: f32) -> Self {
        self.scores.fts = Some(score);
        self
    }

    pub const fn update_fused(&mut self, fused: f32) {
        self.fused = fused;
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

pub const fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
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

pub fn sort_by_fused_desc<T>(items: &mut [Scored<T>])
where
    T: RetrievalCandidate,
{
    items.sort_by(|a, b| {
        b.fused
            .partial_cmp(&a.fused)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.item.candidate_id().cmp(b.item.candidate_id()))
    });
}

/// Fuse two ranked candidate lists into a single ranking using reciprocal rank fusion.
///
/// This is the sole fusion mechanism for the retrieval pipeline: vector and full-text
/// candidates each contribute `weight / (k + rank + 1)` to a shared fused score.
pub fn reciprocal_rank_fusion<T>(
    mut vector_ranked: Vec<Scored<T>>,
    mut fts_ranked: Vec<Scored<T>>,
    config: RrfConfig,
) -> Vec<Scored<T>>
where
    T: RetrievalCandidate,
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
                .then_with(|| a.item.candidate_id().cmp(b.item.candidate_id()))
        });

        for (rank, candidate) in vector_ranked.into_iter().enumerate() {
            let id = candidate.item.candidate_id().to_owned();
            let rank_f32: f32 = u16::try_from(rank).map_or(f32::MAX, f32::from);
            let contribution = vector_weight / (k + rank_f32 + 1.0);

            match merged.entry(id) {
                Entry::Occupied(mut occupied) => {
                    let entry = occupied.get_mut();
                    if let Some(score) = candidate.scores.vector {
                        let existing = entry.scores.vector.unwrap_or(f32::MIN);
                        if score > existing {
                            entry.scores.vector = Some(score);
                        }
                    }
                    entry.item = candidate.item;
                    entry.fused += contribution;
                }
                Entry::Vacant(vacant) => {
                    let mut scored = Scored::new(candidate.item);
                    if let Some(score) = candidate.scores.vector {
                        scored.scores.vector = Some(score);
                    }
                    scored.fused = contribution;
                    vacant.insert(scored);
                }
            }
        }
    }

    if config.use_fts && !fts_ranked.is_empty() {
        fts_ranked.sort_by(|a, b| {
            let a_score = a.scores.fts.unwrap_or(0.0);
            let b_score = b.scores.fts.unwrap_or(0.0);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.item.candidate_id().cmp(b.item.candidate_id()))
        });

        for (rank, candidate) in fts_ranked.into_iter().enumerate() {
            let id = candidate.item.candidate_id().to_owned();
            let rank_f32: f32 = u16::try_from(rank).map_or(f32::MAX, f32::from);
            let contribution = fts_weight / (k + rank_f32 + 1.0);

            match merged.entry(id) {
                Entry::Occupied(mut occupied) => {
                    let entry = occupied.get_mut();
                    if let Some(score) = candidate.scores.fts {
                        let existing = entry.scores.fts.unwrap_or(f32::MIN);
                        if score > existing {
                            entry.scores.fts = Some(score);
                        }
                    }
                    entry.item = candidate.item;
                    entry.fused += contribution;
                }
                Entry::Vacant(vacant) => {
                    let mut scored = Scored::new(candidate.item);
                    if let Some(score) = candidate.scores.fts {
                        scored.scores.fts = Some(score);
                    }
                    scored.fused = contribution;
                    vacant.insert(scored);
                }
            }
        }
    }

    let mut fused: Vec<Scored<T>> = merged.into_values().collect();
    sort_by_fused_desc(&mut fused);
    fused
}
