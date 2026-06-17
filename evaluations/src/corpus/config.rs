use std::path::PathBuf;

use crate::args::Config;

#[derive(Debug, Clone)]
pub struct CorpusCacheConfig {
    pub ingestion_cache_dir: PathBuf,
    pub force_refresh: bool,
    pub refresh_embeddings_only: bool,
    pub ingestion_batch_size: usize,
    pub ingestion_max_retries: usize,
}

impl From<&Config> for CorpusCacheConfig {
    fn from(config: &Config) -> Self {
        Self {
            ingestion_cache_dir: config.ingest.ingestion_cache_dir.clone(),
            force_refresh: config.force_convert || config.ingest.slice_reset_ingestion,
            refresh_embeddings_only: config.ingest.refresh_embeddings_only,
            ingestion_batch_size: config.ingest.ingestion_batch_size,
            ingestion_max_retries: config.ingest.ingestion_max_retries,
        }
    }
}
