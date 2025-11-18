use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::fs;

use crate::{args::Config, embedding::EmbeddingProvider, slice};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub dataset_id: String,
    pub slice_id: String,
    pub embedding_backend: String,
    pub embedding_model: Option<String>,
    pub embedding_dimension: usize,
    pub chunk_min_chars: usize,
    pub chunk_max_chars: usize,
    pub rerank_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSnapshotState {
    pub dataset_id: String,
    pub slice_id: String,
    pub ingestion_fingerprint: String,
    pub snapshot_hash: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub slice_case_count: usize,
}

pub struct Descriptor {
    #[allow(dead_code)]
    metadata: SnapshotMetadata,
    dir: PathBuf,
    metadata_hash: String,
}

impl Descriptor {
    pub fn new(
        config: &Config,
        slice: &slice::ResolvedSlice<'_>,
        embedding_provider: &EmbeddingProvider,
    ) -> Self {
        let metadata = SnapshotMetadata {
            dataset_id: slice.manifest.dataset_id.clone(),
            slice_id: slice.manifest.slice_id.clone(),
            embedding_backend: embedding_provider.backend_label().to_string(),
            embedding_model: embedding_provider.model_code(),
            embedding_dimension: embedding_provider.dimension(),
            chunk_min_chars: config.retrieval.chunk_min_chars,
            chunk_max_chars: config.retrieval.chunk_max_chars,
            rerank_enabled: config.retrieval.rerank,
        };

        let dir = config
            .cache_dir
            .join("snapshots")
            .join(&metadata.dataset_id)
            .join(&metadata.slice_id);
        let metadata_hash = compute_hash(&metadata);

        Self {
            metadata,
            dir,
            metadata_hash,
        }
    }

    pub fn metadata_hash(&self) -> &str {
        &self.metadata_hash
    }

    pub async fn load_db_state(&self) -> Result<Option<DbSnapshotState>> {
        let path = self.db_state_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .await
            .with_context(|| format!("reading namespace state {}", path.display()))?;
        let state = serde_json::from_slice(&bytes)
            .with_context(|| format!("deserialising namespace state {}", path.display()))?;
        Ok(Some(state))
    }

    pub async fn store_db_state(&self, state: &DbSnapshotState) -> Result<()> {
        let path = self.db_state_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.with_context(|| {
                format!("creating namespace state directory {}", parent.display())
            })?;
        }
        let blob =
            serde_json::to_vec_pretty(state).context("serialising namespace state payload")?;
        fs::write(&path, blob)
            .await
            .with_context(|| format!("writing namespace state {}", path.display()))?;
        Ok(())
    }

    fn db_dir(&self) -> PathBuf {
        self.dir.join("db")
    }

    fn db_state_path(&self) -> PathBuf {
        self.db_dir().join("state.json")
    }

    #[cfg(test)]
    pub fn from_parts(metadata: SnapshotMetadata, dir: PathBuf) -> Self {
        let metadata_hash = compute_hash(&metadata);
        Self {
            metadata,
            dir,
            metadata_hash,
        }
    }
}

fn compute_hash(metadata: &SnapshotMetadata) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_vec(metadata).expect("snapshot metadata serialisation should succeed"),
    );
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn state_round_trip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let metadata = SnapshotMetadata {
            dataset_id: "dataset".into(),
            slice_id: "slice".into(),
            embedding_backend: "hashed".into(),
            embedding_model: None,
            embedding_dimension: 128,
            chunk_min_chars: 10,
            chunk_max_chars: 100,
            rerank_enabled: true,
        };
        let descriptor = Descriptor::from_parts(
            metadata,
            temp_dir
                .path()
                .join("snapshots")
                .join("dataset")
                .join("slice"),
        );

        let state = DbSnapshotState {
            dataset_id: "dataset".into(),
            slice_id: "slice".into(),
            ingestion_fingerprint: "fingerprint".into(),
            snapshot_hash: descriptor.metadata_hash().to_string(),
            updated_at: Utc::now(),
            namespace: Some("ns".into()),
            database: Some("db".into()),
            slice_case_count: 42,
        };
        descriptor.store_db_state(&state).await.unwrap();

        let loaded = descriptor.load_db_state().await.unwrap().unwrap();
        assert_eq!(loaded.dataset_id, state.dataset_id);
        assert_eq!(loaded.slice_id, state.slice_id);
        assert_eq!(loaded.ingestion_fingerprint, state.ingestion_fingerprint);
        assert_eq!(loaded.snapshot_hash, state.snapshot_hash);
        assert_eq!(loaded.namespace, state.namespace);
        assert_eq!(loaded.database, state.database);
        assert_eq!(loaded.slice_case_count, state.slice_case_count);
    }
}
