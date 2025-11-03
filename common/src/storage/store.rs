use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
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

/// Storage manager with persistent state and proper lifecycle management.
#[derive(Clone)]
pub struct StorageManager {
    store: DynStore,
    backend_kind: StorageKind,
    local_base: Option<PathBuf>,
}

impl StorageManager {
    /// Create a new StorageManager with the specified configuration.
    ///
    /// This method validates the configuration and creates the appropriate
    /// storage backend with proper initialization.
    pub async fn new(cfg: &AppConfig) -> object_store::Result<Self> {
        let backend_kind = cfg.storage.clone();
        let (store, local_base) = create_storage_backend(cfg).await?;

        Ok(Self {
            store,
            backend_kind,
            local_base,
        })
    }

    /// Create a StorageManager with a custom storage backend.
    ///
    /// This method is useful for testing scenarios where you want to inject
    /// a specific storage backend.
    pub fn with_backend(store: DynStore, backend_kind: StorageKind) -> Self {
        Self {
            store,
            backend_kind,
            local_base: None,
        }
    }

    /// Get the storage backend kind.
    pub fn backend_kind(&self) -> &StorageKind {
        &self.backend_kind
    }

    /// Access the resolved local base directory when using the local backend.
    pub fn local_base_path(&self) -> Option<&Path> {
        self.local_base.as_deref()
    }

    /// Resolve an object location to a filesystem path when using the local backend.
    ///
    /// Returns `None` when the backend is not local or when the provided location includes
    /// unsupported components (absolute paths or parent traversals).
    pub fn resolve_local_path(&self, location: &str) -> Option<PathBuf> {
        let base = self.local_base_path()?;
        let relative = Path::new(location);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return None;
        }

        Some(base.join(relative))
    }

    /// Store bytes at the specified location.
    ///
    /// This operation persists data using the underlying storage backend.
    /// For memory backends, data persists for the lifetime of the StorageManager.
    pub async fn put(&self, location: &str, data: Bytes) -> object_store::Result<()> {
        let path = ObjPath::from(location);
        let payload = object_store::PutPayload::from_bytes(data);
        self.store.put(&path, payload).await.map(|_| ())
    }

    /// Retrieve bytes from the specified location.
    ///
    /// Returns the full contents buffered in memory.
    pub async fn get(&self, location: &str) -> object_store::Result<Bytes> {
        let path = ObjPath::from(location);
        let result = self.store.get(&path).await?;
        result.bytes().await
    }

    /// Get a streaming handle for large objects.
    ///
    /// Returns a fallible stream of Bytes chunks suitable for large file processing.
    pub async fn get_stream(
        &self,
        location: &str,
    ) -> object_store::Result<BoxStream<'static, object_store::Result<Bytes>>> {
        let path = ObjPath::from(location);
        let result = self.store.get(&path).await?;
        Ok(result.into_stream())
    }

    /// Delete all objects below the specified prefix.
    ///
    /// For local filesystem backends, this also attempts to clean up empty directories.
    pub async fn delete_prefix(&self, prefix: &str) -> object_store::Result<()> {
        let prefix_path = ObjPath::from(prefix);
        let locations = self
            .store
            .list(Some(&prefix_path))
            .map_ok(|m| m.location)
            .boxed();
        self.store
            .delete_stream(locations)
            .try_collect::<Vec<_>>()
            .await?;

        // Cleanup filesystem directories only for local backend
        if matches!(self.backend_kind, StorageKind::Local) {
            self.cleanup_filesystem_directories(prefix).await?;
        }

        Ok(())
    }

    /// List all objects below the specified prefix.
    pub async fn list(
        &self,
        prefix: Option<&str>,
    ) -> object_store::Result<Vec<object_store::ObjectMeta>> {
        let prefix_path = prefix.map(ObjPath::from);
        self.store.list(prefix_path.as_ref()).try_collect().await
    }

    /// Check if an object exists at the specified location.
    pub async fn exists(&self, location: &str) -> object_store::Result<bool> {
        let path = ObjPath::from(location);
        self.store
            .head(&path)
            .await
            .map(|_| true)
            .or_else(|e| match e {
                object_store::Error::NotFound { .. } => Ok(false),
                _ => Err(e),
            })
    }

    /// Cleanup filesystem directories for local backend.
    ///
    /// This is a best-effort cleanup and ignores errors.
    async fn cleanup_filesystem_directories(&self, prefix: &str) -> object_store::Result<()> {
        if !matches!(self.backend_kind, StorageKind::Local) {
            return Ok(());
        }

        let Some(base) = &self.local_base else {
            return Ok(());
        };

        let relative = Path::new(prefix);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            tracing::warn!(
                prefix = %prefix,
                "Skipping directory cleanup for unsupported prefix components"
            );
            return Ok(());
        }

        let mut current = base.join(relative);

        while current.starts_with(base) && current.as_path() != base.as_path() {
            match tokio::fs::remove_dir(&current).await {
                Ok(_) => {}
                Err(err) => match err.kind() {
                    ErrorKind::NotFound => {}
                    ErrorKind::DirectoryNotEmpty => break,
                    _ => tracing::debug!(
                        error = %err,
                        path = %current.display(),
                        "Failed to remove directory during cleanup"
                    ),
                },
            }

            if let Some(parent) = current.parent() {
                current = parent.to_path_buf();
            } else {
                break;
            }
        }

        Ok(())
    }
}

/// Create a storage backend based on configuration.
///
/// This factory function handles the creation and initialization of different
/// storage backends with proper error handling and validation.
async fn create_storage_backend(
    cfg: &AppConfig,
) -> object_store::Result<(DynStore, Option<PathBuf>)> {
    match cfg.storage {
        StorageKind::Local => {
            let base = resolve_base_dir(cfg);
            if !base.exists() {
                tokio::fs::create_dir_all(&base).await.map_err(|e| {
                    object_store::Error::Generic {
                        store: "LocalFileSystem",
                        source: e.into(),
                    }
                })?;
            }
            let store = LocalFileSystem::new_with_prefix(base.clone())?;
            Ok((Arc::new(store), Some(base)))
        }
        StorageKind::Memory => {
            let store = InMemory::new();
            Ok((Arc::new(store), None))
        }
    }
}

/// Testing utilities for storage operations.
///
/// This module provides specialized utilities for testing scenarios with
/// automatic memory backend setup and proper test isolation.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::utils::config::{AppConfig, PdfIngestMode};
    use uuid;

    /// Create a test configuration with memory storage.
    ///
    /// This provides a ready-to-use configuration for testing scenarios
    /// that don't require filesystem persistence.
    pub fn test_config_memory() -> AppConfig {
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
            pdf_ingest_mode: PdfIngestMode::LlmFirst,
            ..Default::default()
        }
    }

    /// Create a test configuration with local storage.
    ///
    /// This provides a ready-to-use configuration for testing scenarios
    /// that require actual filesystem operations.
    pub fn test_config_local() -> AppConfig {
        let base = format!("/tmp/minne_test_storage_{}", uuid::Uuid::new_v4());
        AppConfig {
            openai_api_key: "test".into(),
            surrealdb_address: "test".into(),
            surrealdb_username: "test".into(),
            surrealdb_password: "test".into(),
            surrealdb_namespace: "test".into(),
            surrealdb_database: "test".into(),
            data_dir: base.into(),
            http_port: 0,
            openai_base_url: "..".into(),
            storage: StorageKind::Local,
            pdf_ingest_mode: PdfIngestMode::LlmFirst,
            ..Default::default()
        }
    }

    /// A specialized StorageManager for testing scenarios.
    ///
    /// This provides automatic setup for memory storage with proper isolation
    /// and cleanup capabilities for test environments.
    #[derive(Clone)]
    pub struct TestStorageManager {
        storage: StorageManager,
        _temp_dir: Option<(String, std::path::PathBuf)>, // For local storage cleanup
    }

    impl TestStorageManager {
        /// Create a new TestStorageManager with memory backend.
        ///
        /// This is the preferred method for unit tests as it provides
        /// fast execution and complete isolation.
        pub async fn new_memory() -> object_store::Result<Self> {
            let cfg = test_config_memory();
            let storage = StorageManager::new(&cfg).await?;

            Ok(Self {
                storage,
                _temp_dir: None,
            })
        }

        /// Create a new TestStorageManager with local filesystem backend.
        ///
        /// This method creates a temporary directory that will be automatically
        /// cleaned up when the TestStorageManager is dropped.
        pub async fn new_local() -> object_store::Result<Self> {
            let cfg = test_config_local();
            let storage = StorageManager::new(&cfg).await?;
            let resolved = storage
                .local_base_path()
                .map(|path| (cfg.data_dir.clone(), path.to_path_buf()));

            Ok(Self {
                storage,
                _temp_dir: resolved,
            })
        }

        /// Create a TestStorageManager with custom configuration.
        pub async fn with_config(cfg: &AppConfig) -> object_store::Result<Self> {
            let storage = StorageManager::new(cfg).await?;
            let temp_dir = if matches!(cfg.storage, StorageKind::Local) {
                storage
                    .local_base_path()
                    .map(|path| (cfg.data_dir.clone(), path.to_path_buf()))
            } else {
                None
            };

            Ok(Self {
                storage,
                _temp_dir: temp_dir,
            })
        }

        /// Get a reference to the underlying StorageManager.
        pub fn storage(&self) -> &StorageManager {
            &self.storage
        }

        /// Clone the underlying StorageManager.
        pub fn clone_storage(&self) -> StorageManager {
            self.storage.clone()
        }

        /// Store test data at the specified location.
        pub async fn put(&self, location: &str, data: &[u8]) -> object_store::Result<()> {
            self.storage.put(location, Bytes::from(data.to_vec())).await
        }

        /// Retrieve test data from the specified location.
        pub async fn get(&self, location: &str) -> object_store::Result<Bytes> {
            self.storage.get(location).await
        }

        /// Delete test data below the specified prefix.
        pub async fn delete_prefix(&self, prefix: &str) -> object_store::Result<()> {
            self.storage.delete_prefix(prefix).await
        }

        /// Check if test data exists at the specified location.
        pub async fn exists(&self, location: &str) -> object_store::Result<bool> {
            self.storage.exists(location).await
        }

        /// List all test objects below the specified prefix.
        pub async fn list(
            &self,
            prefix: Option<&str>,
        ) -> object_store::Result<Vec<object_store::ObjectMeta>> {
            self.storage.list(prefix).await
        }
    }

    impl Drop for TestStorageManager {
        fn drop(&mut self) {
            // Clean up temporary directories for local storage
            if let Some((_, path)) = &self._temp_dir {
                if path.exists() {
                    let _ = std::fs::remove_dir_all(path);
                }
            }
        }
    }

    /// Convenience macro for creating memory storage tests.
    ///
    /// This macro simplifies the creation of test storage with memory backend.
    #[macro_export]
    macro_rules! test_storage_memory {
        () => {{
            async move {
                $crate::storage::store::testing::TestStorageManager::new_memory()
                    .await
                    .expect("Failed to create test memory storage")
            }
        }};
    }

    /// Convenience macro for creating local storage tests.
    ///
    /// This macro simplifies the creation of test storage with local filesystem backend.
    #[macro_export]
    macro_rules! test_storage_local {
        () => {{
            async move {
                $crate::storage::store::testing::TestStorageManager::new_local()
                    .await
                    .expect("Failed to create test local storage")
            }
        }};
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
    async fn test_storage_manager_memory_basic_operations() {
        let cfg = test_config_memory();
        let storage = StorageManager::new(&cfg)
            .await
            .expect("create storage manager");
        assert!(storage.local_base_path().is_none());

        let location = "test/data/file.txt";
        let data = b"test data for storage manager";

        // Test put and get
        storage
            .put(location, Bytes::from(data.to_vec()))
            .await
            .expect("put");
        let retrieved = storage.get(location).await.expect("get");
        assert_eq!(retrieved.as_ref(), data);

        // Test exists
        assert!(storage.exists(location).await.expect("exists check"));

        // Test delete
        storage.delete_prefix("test/data/").await.expect("delete");
        assert!(!storage
            .exists(location)
            .await
            .expect("exists check after delete"));
    }

    #[tokio::test]
    async fn test_storage_manager_local_basic_operations() {
        let base = format!("/tmp/minne_storage_test_{}", Uuid::new_v4());
        let cfg = test_config(&base);
        let storage = StorageManager::new(&cfg)
            .await
            .expect("create storage manager");
        let resolved_base = storage
            .local_base_path()
            .expect("resolved base dir")
            .to_path_buf();
        assert_eq!(resolved_base, PathBuf::from(&base));

        let location = "test/data/file.txt";
        let data = b"test data for local storage";

        // Test put and get
        storage
            .put(location, Bytes::from(data.to_vec()))
            .await
            .expect("put");
        let retrieved = storage.get(location).await.expect("get");
        assert_eq!(retrieved.as_ref(), data);

        let object_dir = resolved_base.join("test/data");
        tokio::fs::metadata(&object_dir)
            .await
            .expect("object directory exists after write");

        // Test exists
        assert!(storage.exists(location).await.expect("exists check"));

        // Test delete
        storage.delete_prefix("test/data/").await.expect("delete");
        assert!(!storage
            .exists(location)
            .await
            .expect("exists check after delete"));
        assert!(
            tokio::fs::metadata(&object_dir).await.is_err(),
            "object directory should be removed"
        );
        tokio::fs::metadata(&resolved_base)
            .await
            .expect("base directory remains intact");

        // Clean up
        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn test_storage_manager_memory_persistence() {
        let cfg = test_config_memory();
        let storage = StorageManager::new(&cfg)
            .await
            .expect("create storage manager");

        let location = "persistence/test.txt";
        let data1 = b"first data";
        let data2 = b"second data";

        // Put first data
        storage
            .put(location, Bytes::from(data1.to_vec()))
            .await
            .expect("put first");

        // Retrieve and verify first data
        let retrieved1 = storage.get(location).await.expect("get first");
        assert_eq!(retrieved1.as_ref(), data1);

        // Overwrite with second data
        storage
            .put(location, Bytes::from(data2.to_vec()))
            .await
            .expect("put second");

        // Retrieve and verify second data
        let retrieved2 = storage.get(location).await.expect("get second");
        assert_eq!(retrieved2.as_ref(), data2);

        // Data persists across multiple operations using the same StorageManager
        assert_ne!(retrieved1.as_ref(), retrieved2.as_ref());
    }

    #[tokio::test]
    async fn test_storage_manager_list_operations() {
        let cfg = test_config_memory();
        let storage = StorageManager::new(&cfg)
            .await
            .expect("create storage manager");

        // Create multiple files
        let files = vec![
            ("dir1/file1.txt", b"content1"),
            ("dir1/file2.txt", b"content2"),
            ("dir2/file3.txt", b"content3"),
        ];

        for (location, data) in &files {
            storage
                .put(location, Bytes::from(data.to_vec()))
                .await
                .expect("put");
        }

        // Test listing without prefix
        let all_files = storage.list(None).await.expect("list all");
        assert_eq!(all_files.len(), 3);

        // Test listing with prefix
        let dir1_files = storage.list(Some("dir1/")).await.expect("list dir1");
        assert_eq!(dir1_files.len(), 2);
        assert!(dir1_files
            .iter()
            .any(|meta| meta.location.as_ref().contains("file1.txt")));
        assert!(dir1_files
            .iter()
            .any(|meta| meta.location.as_ref().contains("file2.txt")));

        // Test listing non-existent prefix
        let empty_files = storage
            .list(Some("nonexistent/"))
            .await
            .expect("list nonexistent");
        assert_eq!(empty_files.len(), 0);
    }

    #[tokio::test]
    async fn test_storage_manager_stream_operations() {
        let cfg = test_config_memory();
        let storage = StorageManager::new(&cfg)
            .await
            .expect("create storage manager");

        let location = "stream/test.bin";
        let content = vec![42u8; 1024 * 64]; // 64KB of data

        // Put large data
        storage
            .put(location, Bytes::from(content.clone()))
            .await
            .expect("put large data");

        // Get as stream
        let mut stream = storage.get_stream(location).await.expect("get stream");
        let mut collected = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("stream chunk");
            collected.extend_from_slice(&chunk);
        }

        assert_eq!(collected, content);
    }

    #[tokio::test]
    async fn test_storage_manager_with_custom_backend() {
        use object_store::memory::InMemory;

        // Create custom memory backend
        let custom_store = InMemory::new();
        let storage = StorageManager::with_backend(Arc::new(custom_store), StorageKind::Memory);

        let location = "custom/test.txt";
        let data = b"custom backend test";

        // Test operations with custom backend
        storage
            .put(location, Bytes::from(data.to_vec()))
            .await
            .expect("put");
        let retrieved = storage.get(location).await.expect("get");
        assert_eq!(retrieved.as_ref(), data);

        assert!(storage.exists(location).await.expect("exists"));
        assert_eq!(*storage.backend_kind(), StorageKind::Memory);
    }

    #[tokio::test]
    async fn test_storage_manager_error_handling() {
        let cfg = test_config_memory();
        let storage = StorageManager::new(&cfg)
            .await
            .expect("create storage manager");

        // Test getting non-existent file
        let result = storage.get("nonexistent.txt").await;
        assert!(result.is_err());

        // Test checking existence of non-existent file
        let exists = storage
            .exists("nonexistent.txt")
            .await
            .expect("exists check");
        assert!(!exists);

        // Test listing with invalid location (should not panic)
        let _result = storage.get("").await;
        // This may or may not error depending on the backend implementation
        // The important thing is that it doesn't panic
    }

    // TestStorageManager tests
    #[tokio::test]
    async fn test_test_storage_manager_memory() {
        let test_storage = testing::TestStorageManager::new_memory()
            .await
            .expect("create test storage");

        let location = "test/storage/file.txt";
        let data = b"test data with TestStorageManager";

        // Test put and get
        test_storage.put(location, data).await.expect("put");
        let retrieved = test_storage.get(location).await.expect("get");
        assert_eq!(retrieved.as_ref(), data);

        // Test existence check
        assert!(test_storage.exists(location).await.expect("exists"));

        // Test list
        let files = test_storage
            .list(Some("test/storage/"))
            .await
            .expect("list");
        assert_eq!(files.len(), 1);

        // Test delete
        test_storage
            .delete_prefix("test/storage/")
            .await
            .expect("delete");
        assert!(!test_storage
            .exists(location)
            .await
            .expect("exists after delete"));
    }

    #[tokio::test]
    async fn test_test_storage_manager_local() {
        let test_storage = testing::TestStorageManager::new_local()
            .await
            .expect("create test storage");

        let location = "test/local/file.txt";
        let data = b"test data with local TestStorageManager";

        // Test put and get
        test_storage.put(location, data).await.expect("put");
        let retrieved = test_storage.get(location).await.expect("get");
        assert_eq!(retrieved.as_ref(), data);

        // Test existence check
        assert!(test_storage.exists(location).await.expect("exists"));

        // The storage should be automatically cleaned up when test_storage is dropped
    }

    #[tokio::test]
    async fn test_test_storage_manager_isolation() {
        let storage1 = testing::TestStorageManager::new_memory()
            .await
            .expect("create test storage 1");
        let storage2 = testing::TestStorageManager::new_memory()
            .await
            .expect("create test storage 2");

        let location = "isolation/test.txt";
        let data1 = b"storage 1 data";
        let data2 = b"storage 2 data";

        // Put different data in each storage
        storage1.put(location, data1).await.expect("put storage 1");
        storage2.put(location, data2).await.expect("put storage 2");

        // Verify isolation
        let retrieved1 = storage1.get(location).await.expect("get storage 1");
        let retrieved2 = storage2.get(location).await.expect("get storage 2");

        assert_eq!(retrieved1.as_ref(), data1);
        assert_eq!(retrieved2.as_ref(), data2);
        assert_ne!(retrieved1.as_ref(), retrieved2.as_ref());
    }

    #[tokio::test]
    async fn test_test_storage_manager_config() {
        let cfg = testing::test_config_memory();
        let test_storage = testing::TestStorageManager::with_config(&cfg)
            .await
            .expect("create test storage with config");

        let location = "config/test.txt";
        let data = b"test data with custom config";

        test_storage.put(location, data).await.expect("put");
        let retrieved = test_storage.get(location).await.expect("get");
        assert_eq!(retrieved.as_ref(), data);

        // Verify it's using memory backend
        assert_eq!(*test_storage.storage().backend_kind(), StorageKind::Memory);
    }
}
