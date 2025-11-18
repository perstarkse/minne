use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;

use crate::{args::Config, embedding::EmbeddingProvider};

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

#[async_trait]
pub trait CorpusEmbeddingProvider: Send + Sync {
    fn backend_label(&self) -> &str;
    fn model_code(&self) -> Option<String>;
    fn dimension(&self) -> usize;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
}

#[async_trait]
impl CorpusEmbeddingProvider for EmbeddingProvider {
    fn backend_label(&self) -> &str {
        EmbeddingProvider::backend_label(self)
    }

    fn model_code(&self) -> Option<String> {
        EmbeddingProvider::model_code(self)
    }

    fn dimension(&self) -> usize {
        EmbeddingProvider::dimension(self)
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        EmbeddingProvider::embed_batch(self, texts).await
    }
}

impl From<&Config> for CorpusCacheConfig {
    fn from(config: &Config) -> Self {
        CorpusCacheConfig::new(
            config.ingestion_cache_dir.clone(),
            config.force_convert || config.slice_reset_ingestion,
            config.refresh_embeddings_only,
            config.ingestion_batch_size,
            config.ingestion_max_retries,
        )
    }
}
