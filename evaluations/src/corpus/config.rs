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

impl CorpusCacheConfig {
    pub fn new(
        ingestion_cache_dir: impl Into<PathBuf>,
        force_refresh: bool,
        refresh_embeddings_only: bool,
        ingestion_batch_size: usize,
        ingestion_max_retries: usize,
    ) -> Self {
        Self {
            ingestion_cache_dir: ingestion_cache_dir.into(),
            force_refresh,
            refresh_embeddings_only,
            ingestion_batch_size,
            ingestion_max_retries,
        }
    }
}

impl From<&Config> for CorpusCacheConfig {
    fn from(config: &Config) -> Self {
        CorpusCacheConfig::new(
            config.ingest.ingestion_cache_dir.clone(),
            config.force_convert || config.ingest.slice_reset_ingestion,
            config.ingest.refresh_embeddings_only,
            config.ingest.ingestion_batch_size,
            config.ingest.ingestion_max_retries,
        )
    }
}
