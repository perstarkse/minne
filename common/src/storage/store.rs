use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result as AnyResult};
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use futures::stream::BoxStream;
use object_store::local::LocalFileSystem;
use object_store::{path::Path as ObjPath, ObjectStore};

use crate::utils::config::{AppConfig, StorageKind};

pub type DynStore = Arc<dyn ObjectStore>;

/// Build an object store instance anchored at the given filesystem `prefix`.
///
/// - For the `Local` backend, `prefix` is the absolute directory on disk that
///   serves as the root for all object paths passed to the store.
/// - `prefix` must already exist; this function will create it if missing.
///
/// Example (Local):
/// - prefix: `/var/data`
/// - object location: `user/uuid/file.txt`
/// - absolute path: `/var/data/user/uuid/file.txt`
pub async fn build_store(prefix: &Path, cfg: &AppConfig) -> object_store::Result<DynStore> {
    match cfg.storage {
        StorageKind::Local => {
            if !prefix.exists() {
                tokio::fs::create_dir_all(prefix)
                    .await
                    .map_err(|e| object_store::Error::Generic {
                        store: "LocalFileSystem",
                        source: e.into(),
                    })?;
            }
            let store = LocalFileSystem::new_with_prefix(prefix)?;
            Ok(Arc::new(store))
        }
    }
}

/// Resolve the absolute base directory used for local storage from config.
///
/// If `data_dir` is relative, it is resolved against the current working directory.
pub fn resolve_base_dir(cfg: &AppConfig) -> PathBuf {
    if cfg.data_dir.starts_with('/') {
        PathBuf::from(&cfg.data_dir)
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            .join(&cfg.data_dir)
    }
}

/// Build an object store rooted at the configured data directory.
///
/// This is the recommended way to obtain a store for logical object operations
/// such as `put_bytes_at`, `get_bytes_at`, and `delete_prefix_at`.
pub async fn build_store_root(cfg: &AppConfig) -> object_store::Result<DynStore> {
    let base = resolve_base_dir(cfg);
    build_store(&base, cfg).await
}

/// Write bytes to `file_name` within a filesystem `prefix` using the configured store.
///
/// Prefer [`put_bytes_at`] for location-based writes that do not need to compute
/// a separate filesystem prefix.
pub async fn put_bytes(prefix: &Path, file_name: &str, data: Bytes, cfg: &AppConfig) -> object_store::Result<()> {
    let store = build_store(prefix, cfg).await?;
    let payload = object_store::PutPayload::from_bytes(data);
    store.put(&ObjPath::from(file_name), payload).await?;
    Ok(())
}

/// Write bytes to the provided logical object `location`, e.g. `"user/uuid/file"`.
///
/// The store root is taken from `AppConfig::data_dir` for the local backend.
/// This performs an atomic write as guaranteed by `object_store`.
pub async fn put_bytes_at(location: &str, data: Bytes, cfg: &AppConfig) -> object_store::Result<()> {
    let store = build_store_root(cfg).await?;
    let payload = object_store::PutPayload::from_bytes(data);
    store.put(&ObjPath::from(location), payload).await?;
    Ok(())
}

/// Read bytes from `file_name` within a filesystem `prefix` using the configured store.
///
/// Prefer [`get_bytes_at`] for location-based reads.
pub async fn get_bytes(prefix: &Path, file_name: &str, cfg: &AppConfig) -> object_store::Result<Bytes> {
    let store = build_store(prefix, cfg).await?;
    let r = store.get(&ObjPath::from(file_name)).await?;
    let b = r.bytes().await?;
    Ok(b)
}

/// Read bytes from the provided logical object `location`.
///
/// Returns the full contents buffered in memory.
pub async fn get_bytes_at(location: &str, cfg: &AppConfig) -> object_store::Result<Bytes> {
    let store = build_store_root(cfg).await?;
    let r = store.get(&ObjPath::from(location)).await?;
    r.bytes().await
}

/// Get a streaming body for the provided logical object `location`.
///
/// Returns a fallible `BoxStream` of `Bytes`, suitable for use with
/// `axum::body::Body::from_stream` to stream responses without buffering.
pub async fn get_stream_at(location: &str, cfg: &AppConfig) -> object_store::Result<BoxStream<'static, object_store::Result<Bytes>>> {
    let store = build_store_root(cfg).await?;
    let r = store.get(&ObjPath::from(location)).await?;
    Ok(r.into_stream())
}

/// Delete all objects below the provided filesystem `prefix`.
///
/// This is a low-level variant for when a dedicated on-disk prefix is used for a
/// particular object grouping. Prefer [`delete_prefix_at`] for location-based stores.
pub async fn delete_prefix(prefix: &Path, cfg: &AppConfig) -> object_store::Result<()> {
    let store = build_store(prefix, cfg).await?;
    // list everything and delete
    let locations = store.list(None).map_ok(|m| m.location).boxed();
    store.delete_stream(locations).try_collect::<Vec<_>>().await?;
    // Best effort remove the directory itself
    if tokio::fs::try_exists(prefix).await.unwrap_or(false) {
        let _ = tokio::fs::remove_dir_all(prefix).await;
    }
    Ok(())
}

/// Delete all objects below the provided logical object `prefix`, e.g. `"user/uuid/"`.
///
/// After deleting, attempts a best-effort cleanup of the now-empty directory on disk
/// when using the local backend.
pub async fn delete_prefix_at(prefix: &str, cfg: &AppConfig) -> object_store::Result<()> {
    let store = build_store_root(cfg).await?;
    let prefix_path = ObjPath::from(prefix);
    let locations = store.list(Some(&prefix_path)).map_ok(|m| m.location).boxed();
    store.delete_stream(locations).try_collect::<Vec<_>>().await?;
    // Best effort remove empty directory on disk for local storage
    let base_dir = resolve_base_dir(cfg).join(prefix);
    if tokio::fs::try_exists(&base_dir).await.unwrap_or(false) {
        let _ = tokio::fs::remove_dir_all(&base_dir).await;
    }
    Ok(())
}

/// Split an absolute filesystem path into `(parent_dir, file_name)`.
pub fn split_abs_path(path: &str) -> AnyResult<(PathBuf, String)> {
    let pb = PathBuf::from(path);
    let parent = pb
        .parent()
        .ok_or_else(|| anyhow!("Path has no parent: {path}"))?
        .to_path_buf();
    let file = pb
        .file_name()
        .ok_or_else(|| anyhow!("Path has no file name: {path}"))?
        .to_string_lossy()
        .to_string();
    Ok((parent, file))
}

/// Split a logical object location `"a/b/c"` into `("a/b", "c")`.
pub fn split_object_path(path: &str) -> AnyResult<(String, String)> {
    if let Some((p, f)) = path.rsplit_once('/') {
        return Ok((p.to_string(), f.to_string()));
    }
    Err(anyhow!("Object path has no separator: {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::config::StorageKind;
    use bytes::Bytes;
    use futures::TryStreamExt;
    use uuid::Uuid;

    fn test_config(root: &str) -> AppConfig {
        AppConfig {
            openai_api_key: "test".into(),
            surrealdb_address: "test".into(),
            surrealdb_username: "test".into(),
            surrealdb_password: "test".into(),
            surrealdb_namespace: "test".into(),
            surrealdb_database: "test".into(),
            data_dir: root.into(),
            http_port: 0,
            openai_base_url: "..".into(),
            storage: StorageKind::Local,
        }
    }

    #[tokio::test]
    async fn test_build_store_root_creates_base() {
        let base = format!("/tmp/minne_store_test_{}", Uuid::new_v4());
        let cfg = test_config(&base);
        let _ = build_store_root(&cfg).await.expect("build store root");
        assert!(tokio::fs::try_exists(&base).await.unwrap_or(false));
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn test_put_get_bytes_at_and_delete_prefix_at() {
        let base = format!("/tmp/minne_store_test_{}", Uuid::new_v4());
        let cfg = test_config(&base);

        let location_prefix = format!("{}/{}", "user1", Uuid::new_v4());
        let file_name = "file.txt";
        let location = format!("{}/{}", &location_prefix, file_name);
        let payload = Bytes::from_static(b"hello world");

        put_bytes_at(&location, payload.clone(), &cfg).await.expect("put");
        let got = get_bytes_at(&location, &cfg).await.expect("get");
        assert_eq!(got.as_ref(), payload.as_ref());

        // Delete the whole prefix and ensure retrieval fails
        delete_prefix_at(&location_prefix, &cfg).await.expect("delete prefix");
        assert!(get_bytes_at(&location, &cfg).await.is_err());

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn test_get_stream_at() {
        let base = format!("/tmp/minne_store_test_{}", Uuid::new_v4());
        let cfg = test_config(&base);

        let location = format!("{}/{}/stream.bin", "user2", Uuid::new_v4());
        let content = vec![7u8; 32 * 1024]; // 32KB payload

        put_bytes_at(&location, Bytes::from(content.clone()), &cfg)
            .await
            .expect("put");

        let stream = get_stream_at(&location, &cfg).await.expect("stream");
        let combined: Vec<u8> = stream
            .map_ok(|chunk| chunk.to_vec())
            .try_fold(Vec::new(), |mut acc, mut chunk| async move {
                acc.append(&mut chunk);
                Ok(acc)
            })
            .await
            .expect("collect");

        assert_eq!(combined, content);

        delete_prefix_at(
            &split_object_path(&location).unwrap().0,
            &cfg,
        )
        .await
        .ok();

        let _ = tokio::fs::remove_dir_all(&base).await;
    }
}
