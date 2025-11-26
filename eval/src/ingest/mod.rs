mod config;
mod orchestrator;
pub(crate) mod store;

pub use config::CorpusCacheConfig;
pub use orchestrator::{
    cached_corpus_dir, compute_ingestion_fingerprint, corpus_handle_from_manifest, ensure_corpus,
    load_cached_manifest,
};
pub use store::{
    seed_manifest_into_db, window_manifest, CorpusHandle, CorpusManifest, CorpusMetadata,
    CorpusQuestion, EmbeddedKnowledgeEntity, EmbeddedTextChunk, ParagraphShard,
    ParagraphShardStore, MANIFEST_VERSION,
};
