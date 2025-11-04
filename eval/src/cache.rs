use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Debug, Default, Serialize, Deserialize)]
struct EmbeddingCacheData {
    entities: HashMap<String, Vec<f32>>,
    chunks: HashMap<String, Vec<f32>>,
}

#[derive(Clone)]
pub struct EmbeddingCache {
    path: Arc<PathBuf>,
    data: Arc<Mutex<EmbeddingCacheData>>,
    dirty: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl EmbeddingCache {
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let data = if path.exists() {
            let raw = tokio::fs::read(&path)
                .await
                .with_context(|| format!("reading embedding cache {}", path.display()))?;
            serde_json::from_slice(&raw)
                .with_context(|| format!("parsing embedding cache {}", path.display()))?
        } else {
            EmbeddingCacheData::default()
        };

        Ok(Self {
            path: Arc::new(path),
            data: Arc::new(Mutex::new(data)),
            dirty: Arc::new(AtomicBool::new(false)),
        })
    }

    pub async fn get_entity(&self, id: &str) -> Option<Vec<f32>> {
        let guard = self.data.lock().await;
        guard.entities.get(id).cloned()
    }

    pub async fn insert_entity(&self, id: String, embedding: Vec<f32>) {
        let mut guard = self.data.lock().await;
        guard.entities.insert(id, embedding);
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub async fn get_chunk(&self, id: &str) -> Option<Vec<f32>> {
        let guard = self.data.lock().await;
        guard.chunks.get(id).cloned()
    }

    pub async fn insert_chunk(&self, id: String, embedding: Vec<f32>) {
        let mut guard = self.data.lock().await;
        guard.chunks.insert(id, embedding);
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub async fn persist(&self) -> Result<()> {
        if !self.dirty.load(Ordering::Relaxed) {
            return Ok(());
        }

        let guard = self.data.lock().await;
        let body = serde_json::to_vec_pretty(&*guard).context("serialising embedding cache")?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating cache directory {}", parent.display()))?;
        }
        tokio::fs::write(&*self.path, body)
            .await
            .with_context(|| format!("writing embedding cache {}", self.path.display()))?;
        self.dirty.store(false, Ordering::Relaxed);
        Ok(())
    }
}
