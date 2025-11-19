mod config;
mod orchestrator;
pub(crate) mod store;

pub use config::{CorpusCacheConfig, CorpusEmbeddingProvider};
pub use orchestrator::ensure_corpus;
pub use store::{
    seed_manifest_into_db, CorpusHandle, CorpusManifest, CorpusMetadata, CorpusQuestion,
    ParagraphShard, ParagraphShardStore, MANIFEST_VERSION,
};
