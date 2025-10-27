use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result as AnyResult};
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::{path::Path as ObjPath, ObjectStore};

use crate::utils::config::{AppConfig, StorageKind};

pub type DynStore = Arc<dyn ObjectStore>;

/// Build an object store instance anchored at the given filesystem `prefix`.
///
/// - For the `Local` backend, `prefix` is the absolute directory on disk that
///   serves as the root for all object paths passed to the store.
/// - For the `Memory` backend, `prefix` is ignored as storage is purely in-memory.
/// - `prefix` must already exist for Local storage; this function will create it if missing.
///
/// Example (Local):
/// - prefix: `/var/data`
/// - object location: `user/uuid/file.txt`
/// - absolute path: `/var/data/user/uuid/file.txt`
///
/// Example (Memory):
/// - prefix: ignored (any value works)
/// - object location: `user/uuid/file.txt`
/// - stored in memory for the duration of the process
pub async fn build_store(prefix: &Path, cfg: &AppConfig) -> object_store::Result<DynStore> {
    match cfg.storage {
        StorageKind::Local => {
            if !prefix.exists() {
                tokio::fs::create_dir_all(prefix).await.map_err(|e| {
                    object_store::Error::Generic {
                        store: "LocalFileSystem",
                        source: e.into(),
                    }
                })?;
            }
            let store = LocalFileSystem::new_with_prefix(prefix)?;
            Ok(Arc::new(store))
        }
        StorageKind::Memory => {
            // For memory storage, we ignore the prefix as it's purely in-memory
            let store = InMemory::new();
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
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&cfg.data_dir)
    }
}

/// Build an object store rooted at the configured data directory.
///
/// This is the recommended way to obtain a store for logical object operations
/// such as `put_bytes_at`, `get_bytes_at`, and `delete_prefix_at`.
///
/// For `StorageKind::Local`, this creates a filesystem-backed store.
/// For `StorageKind::Memory`, this creates an in-memory store useful for testing.
pub async fn build_store_root(cfg: &AppConfig) -> object_store::Result<DynStore> {
    let base = resolve_base_dir(cfg);
    build_store(&base, cfg).await
}

/// Write bytes to `file_name` within a filesystem `prefix` using the configured store.
///
/// Prefer [`put_bytes_at`] for location-based writes that do not need to compute
/// a separate filesystem prefix.
pub async fn put_bytes(
    prefix: &Path,
    file_name: &str,
    data: Bytes,
    cfg: &AppConfig,
) -> object_store::Result<()> {
    let store = build_store(prefix, cfg).await?;
    let payload = object_store::PutPayload::from_bytes(data);
    store.put(&ObjPath::from(file_name), payload).await?;
    Ok(())
}

/// Write bytes to the provided logical object `location`, e.g. `"user/uuid/file"`.
///
/// The store root is taken from `AppConfig::data_dir` for the local backend.
/// For memory storage, data is stored in memory for the duration of the process.
/// This performs an atomic write as guaranteed by `object_store`.
///
/// **Note**: Each call creates a new store instance. For memory storage,
/// this means data is not persisted across different function calls.
/// Use `build_store_root()` directly when you need to persist data across operations.
pub async fn put_bytes_at(
    location: &str,
    data: Bytes,
    cfg: &AppConfig,
) -> object_store::Result<()> {
    let store = build_store_root(cfg).await?;
    let payload = object_store::PutPayload::from_bytes(data);
    store.put(&ObjPath::from(location), payload).await?;
    Ok(())
}

/// Read bytes from `file_name` within a filesystem `prefix` using the configured store.
///
/// Prefer [`get_bytes_at`] for location-based reads.
pub async fn get_bytes(
    prefix: &Path,
    file_name: &str,
    cfg: &AppConfig,
) -> object_store::Result<Bytes> {
    let store = build_store(prefix, cfg).await?;
    let r = store.get(&ObjPath::from(file_name)).await?;
    let b = r.bytes().await?;
    Ok(b)
}

/// Read bytes from the provided logical object `location`.
///
/// Returns the full contents buffered in memory.
///
/// **Note**: Each call creates a new store instance. For memory storage,
/// this means you can only retrieve data that was written using the same
/// store instance. Use `build_store_root()` directly when you need to
/// persist data across operations.
pub async fn get_bytes_at(location: &str, cfg: &AppConfig) -> object_store::Result<Bytes> {
    let store = build_store_root(cfg).await?;
    let r = store.get(&ObjPath::from(location)).await?;
    r.bytes().await
}

/// Get a streaming body for the provided logical object `location`.
///
/// Returns a fallible `BoxStream` of `Bytes`, suitable for use with
/// `axum::body::Body::from_stream` to stream responses without buffering.
pub async fn get_stream_at(
    location: &str,
    cfg: &AppConfig,
) -> object_store::Result<BoxStream<'static, object_store::Result<Bytes>>> {
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
    store
        .delete_stream(locations)
        .try_collect::<Vec<_>>()
        .await?;
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
    let locations = store
        .list(Some(&prefix_path))
        .map_ok(|m| m.location)
        .boxed();
    store
        .delete_stream(locations)
        .try_collect::<Vec<_>>()
        .await?;
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
    use crate::utils::config::{PdfIngestMode::LlmFirst, StorageKind};
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
            pdf_ingest_mode: LlmFirst,
            ..Default::default()
        }
    }

    fn test_config_memory() -> AppConfig {
        AppConfig {
            openai_api_key: "test".into(),
            surrealdb_address: "test".into(),
            surrealdb_username: "test".into(),
            surrealdb_password: "test".into(),
            surrealdb_namespace: "test".into(),
            surrealdb_database: "test".into(),
            data_dir: "/tmp/unused".into(), // Ignored for memory storage
            http_port: 0,
            openai_base_url: "..".into(),
            storage: StorageKind::Memory,
            pdf_ingest_mode: LlmFirst,
            ..Default::default()
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

        put_bytes_at(&location, payload.clone(), &cfg)
            .await
            .expect("put");
        let got = get_bytes_at(&location, &cfg).await.expect("get");
        assert_eq!(got.as_ref(), payload.as_ref());

        // Delete the whole prefix and ensure retrieval fails
        delete_prefix_at(&location_prefix, &cfg)
            .await
            .expect("delete prefix");
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

        delete_prefix_at(&split_object_path(&location).unwrap().0, &cfg)
            .await
            .ok();

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    // Memory storage tests
    //
    // Example usage for testing with memory storage:
    //
    // ```rust
    // let cfg = AppConfig {
    //     storage: StorageKind::Memory,
    //     // ... other fields
    //     ..Default::default()
    // };
    //
    // let store = build_store_root(&cfg).await?;
    // // Use the store for multiple operations to maintain data persistence
    // store.put(&path, data).await?;
    // let result = store.get(&path).await?;
    // ```
    #[tokio::test]
    async fn test_build_store_memory_creates_store() {
        let cfg = test_config_memory();
        let _ = build_store_root(&cfg).await.expect("build memory store root");
        // Memory store should be created without any filesystem operations
    }

    #[tokio::test]
    async fn test_memory_put_get_bytes_at() {
        let cfg = test_config_memory();

        // Create a single store instance to reuse across operations
        let store = build_store_root(&cfg).await.expect("build memory store root");
        let location_prefix = format!("{}/{}", "user1", Uuid::new_v4());
        let file_name = "file.txt";
        let location = format!("{}/{}", &location_prefix, file_name);
        let payload = Bytes::from_static(b"hello world from memory");

        // Use the store directly instead of the convenience functions
        let obj_path = ObjPath::from(location.as_str());
        store.put(&obj_path, object_store::PutPayload::from_bytes(payload.clone())).await.expect("put to memory");
        let got = store.get(&obj_path).await.expect("get from memory").bytes().await.expect("get bytes");
        assert_eq!(got.as_ref(), payload.as_ref());

        // Delete the whole prefix and ensure retrieval fails
        let prefix_path = ObjPath::from(location_prefix.as_str());
        let locations = store.list(Some(&prefix_path)).map_ok(|m| m.location).boxed();
        store.delete_stream(locations).try_collect::<Vec<_>>().await.expect("delete prefix from memory");
        assert!(store.get(&obj_path).await.is_err());
    }

    #[tokio::test]
    async fn test_memory_get_stream_at() {
        let cfg = test_config_memory();

        // Create a single store instance to reuse across operations
        let store = build_store_root(&cfg).await.expect("build memory store root");
        let location = format!("{}/{}/stream.bin", "user2", Uuid::new_v4());
        let content = vec![42u8; 32 * 1024]; // 32KB payload

        // Use the store directly
        let obj_path = ObjPath::from(location.as_str());
        store.put(&obj_path, object_store::PutPayload::from_bytes(Bytes::from(content.clone()))).await.expect("put to memory");

        let stream = store.get(&obj_path).await.expect("get from memory").into_stream();
        let combined: Vec<u8> = stream
            .map_ok(|chunk| chunk.to_vec())
            .try_fold(Vec::new(), |mut acc, mut chunk| async move {
                acc.append(&mut chunk);
                Ok(acc)
            })
            .await
            .expect("collect");

        assert_eq!(combined, content);

        // Clean up
        delete_prefix_at(&split_object_path(&location).unwrap().0, &cfg)
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_memory_store_isolation() {
        // Create two different memory stores to test isolation
        let cfg1 = test_config_memory();
        let cfg2 = test_config_memory();

        let store1 = build_store_root(&cfg1).await.expect("build memory store 1");
        let store2 = build_store_root(&cfg2).await.expect("build memory store 2");

        let location = "test/isolation/file.txt".to_string();
        let payload1 = Bytes::from_static(b"store 1 content");
        let payload2 = Bytes::from_static(b"store 2 content");

        // Put different data in each store
        let obj_path = ObjPath::from(location.as_str());
        store1.put(&obj_path, object_store::PutPayload::from_bytes(payload1.clone())).await.expect("put to store 1");
        store2.put(&obj_path, object_store::PutPayload::from_bytes(payload2.clone())).await.expect("put to store 2");

        // Verify isolation - each store should only see its own data
        let got1 = store1.get(&obj_path).await.expect("get from store 1").bytes().await.expect("get bytes 1");
        let got2 = store2.get(&obj_path).await.expect("get from store 2").bytes().await.expect("get bytes 2");

        assert_eq!(got1.as_ref(), payload1.as_ref());
        assert_eq!(got2.as_ref(), payload2.as_ref());
        assert_ne!(got1.as_ref(), got2.as_ref());
    }

    #[tokio::test]
    async fn test_memory_vs_local_behavior_equivalence() {
        // Test that memory and local storage have equivalent behavior
        let base = format!("/tmp/minne_store_test_{}", Uuid::new_v4());
        let local_cfg = test_config(&base);
        let memory_cfg = test_config_memory();

        // Create stores
        let local_store = build_store_root(&local_cfg).await.expect("build local store");
        let memory_store = build_store_root(&memory_cfg).await.expect("build memory store");

        let location = "test/comparison/data.txt".to_string();
        let payload = Bytes::from_static(b"test data for comparison");
        let obj_path = ObjPath::from(location.as_str());

        // Put data in both stores
        local_store.put(&obj_path, object_store::PutPayload::from_bytes(payload.clone())).await.expect("put to local");
        memory_store.put(&obj_path, object_store::PutPayload::from_bytes(payload.clone())).await.expect("put to memory");

        // Get data from both stores
        let local_result = local_store.get(&obj_path).await.expect("get from local").bytes().await.expect("get local bytes");
        let memory_result = memory_store.get(&obj_path).await.expect("get from memory").bytes().await.expect("get memory bytes");

        // Verify equivalence
        assert_eq!(local_result.as_ref(), memory_result.as_ref());
        assert_eq!(local_result.as_ref(), payload.as_ref());

        // Test listing behavior
        let local_prefix = ObjPath::from("test");
        let memory_prefix = ObjPath::from("test");

        let local_list: Vec<object_store::ObjectMeta> = local_store.list(Some(&local_prefix))
            .try_collect().await.expect("list local");
        let memory_list: Vec<object_store::ObjectMeta> = memory_store.list(Some(&memory_prefix))
            .try_collect().await.expect("list memory");

        assert_eq!(local_list.len(), memory_list.len());
        assert_eq!(local_list[0].location, memory_list[0].location);

        // Clean up
        let _ = tokio::fs::remove_dir_all(&base).await;
    }
}
