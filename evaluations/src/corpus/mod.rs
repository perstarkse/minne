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

pub fn make_ingestion_config(config: &crate::args::Config) -> ingestion_pipeline::IngestionConfig {
    let mut tuning = ingestion_pipeline::IngestionTuning::default();
    tuning.chunk_min_tokens = config.ingest.ingest_chunk_min_tokens;
    tuning.chunk_max_tokens = config.ingest.ingest_chunk_max_tokens;
    tuning.chunk_overlap_tokens = config.ingest.ingest_chunk_overlap_tokens;

    ingestion_pipeline::IngestionConfig {
        tuning,
        chunk_only: config.ingest.ingest_chunks_only,
    }
}
