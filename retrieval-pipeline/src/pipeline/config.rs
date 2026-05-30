use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Two-variant flag that serializes as a bool for backward compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoolFlag {
    #[default]
    Disabled,
    Enabled,
}

impl BoolFlag {
    pub const fn as_bool(self) -> bool {
        matches!(self, BoolFlag::Enabled)
    }
}

impl From<bool> for BoolFlag {
    fn from(value: bool) -> Self {
        if value {
            BoolFlag::Enabled
        } else {
            BoolFlag::Disabled
        }
    }
}

impl Serialize for BoolFlag {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(self.as_bool())
    }
}

impl<'de> Deserialize<'de> for BoolFlag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        bool::deserialize(deserializer).map(|b| {
            if b {
                BoolFlag::Enabled
            } else {
                BoolFlag::Disabled
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RetrievalTuningFlags {
    pub rerank_scores_only: BoolFlag,
    pub chunk_rrf_use_vector: BoolFlag,
    pub chunk_rrf_use_fts: BoolFlag,
}

impl RetrievalTuningFlags {
    pub const fn rerank_scores_only(self) -> bool {
        self.rerank_scores_only.as_bool()
    }

    pub const fn chunk_rrf_use_vector(self) -> bool {
        self.chunk_rrf_use_vector.as_bool()
    }

    pub const fn chunk_rrf_use_fts(self) -> bool {
        self.chunk_rrf_use_fts.as_bool()
    }
}

impl Default for RetrievalTuningFlags {
    fn default() -> Self {
        Self {
            rerank_scores_only: BoolFlag::Disabled,
            chunk_rrf_use_vector: BoolFlag::Enabled,
            chunk_rrf_use_fts: BoolFlag::Enabled,
        }
    }
}

/// Tunable parameters governing the chunk-first hybrid (vector + FTS, RRF-fused) retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalTuning {
    /// Number of vector candidates to pull from the chunk embedding index.
    pub chunk_vector_take: usize,
    /// Number of full-text candidates to pull from the chunk index.
    pub chunk_fts_take: usize,
    /// Maximum chunks attached to each resolved entity.
    pub max_chunks_per_entity: usize,
    /// Blend weight applied when mixing reranker scores with fused scores.
    pub rerank_blend_weight: f32,
    /// Keep top-N candidates after reranking.
    pub rerank_keep_top: usize,
    /// Maximum number of chunks returned to callers.
    pub chunk_result_cap: usize,
    /// Reciprocal rank fusion k value for chunk merging.
    pub chunk_rrf_k: f32,
    /// Weight applied to vector ranks in RRF.
    pub chunk_rrf_vector_weight: f32,
    /// Weight applied to chunk FTS ranks in RRF.
    pub chunk_rrf_fts_weight: f32,
    pub flags: RetrievalTuningFlags,
}

impl Default for RetrievalTuning {
    fn default() -> Self {
        Self {
            chunk_vector_take: 20,
            chunk_fts_take: 20,
            max_chunks_per_entity: 4,
            rerank_blend_weight: 0.65,
            rerank_keep_top: 8,
            chunk_result_cap: 5,
            chunk_rrf_k: 60.0,
            chunk_rrf_vector_weight: 1.0,
            chunk_rrf_fts_weight: 1.0,
            flags: RetrievalTuningFlags::default(),
        }
    }
}

/// Per-request retrieval configuration.
///
/// The pipeline always performs chunk-first hybrid retrieval. Set `resolve_entities`
/// when a caller additionally needs the `KnowledgeEntity` rows that own the retrieved
/// chunks (search, ingestion linking, relationship suggestion).
#[derive(Debug, Clone, Default)]
pub struct RetrievalConfig {
    pub tuning: RetrievalTuning,
    pub resolve_entities: bool,
}

impl RetrievalConfig {
    /// Chunk retrieval that also resolves the owning knowledge entities.
    pub fn with_entities() -> Self {
        Self {
            tuning: RetrievalTuning::default(),
            resolve_entities: true,
        }
    }
}
