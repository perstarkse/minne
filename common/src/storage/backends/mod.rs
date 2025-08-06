use async_trait::async_trait;
use bytes::Bytes;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

pub mod filesystem;
pub mod object_storage;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Object store error: {0}")]
    ObjectStore(#[from] object_store::Error),
    
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("File not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone)]
pub struct StoredFile {
    pub content: Bytes,
    pub metadata: FileMetadata,
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub size: u64,
    pub content_type: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

/// Trait for abstracting file storage backends
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Store a file and return its storage path/key
    async fn store_file(
        &self,
        user_id: &str,
        file_id: &Uuid,
        filename: &str,
        content: Bytes,
        content_type: Option<&str>,
    ) -> Result<String, StorageError>;

    /// Retrieve a file by its storage path/key
    async fn get_file(&self, path: &str) -> Result<StoredFile, StorageError>;

    /// Delete a file by its storage path/key
    async fn delete_file(&self, path: &str) -> Result<(), StorageError>;

    /// Check if a file exists
    async fn file_exists(&self, path: &str) -> Result<bool, StorageError>;

    /// Get file metadata without downloading the content
    async fn get_metadata(&self, path: &str) -> Result<FileMetadata, StorageError>;

    /// List files with a given prefix (for cleanup or migration)
    async fn list_files(&self, prefix: &str) -> Result<Vec<String>, StorageError>;
}

/// Configuration for different storage backends
#[derive(Debug, Clone)]
pub enum StorageConfig {
    FileSystem {
        data_dir: String,
    },
    S3 {
        bucket: String,
        region: Option<String>,
        endpoint: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        prefix: Option<String>,
    },
}

/// Factory function to create storage backends based on configuration
pub async fn create_storage_backend(config: StorageConfig) -> Result<Box<dyn StorageBackend>, StorageError> {
    match config {
        StorageConfig::FileSystem { data_dir } => {
            Ok(Box::new(filesystem::FilesystemBackend::new(data_dir)?))
        }
        StorageConfig::S3 { 
            bucket, 
            region, 
            endpoint, 
            access_key_id, 
            secret_access_key,
            prefix,
        } => {
            Ok(Box::new(object_storage::S3Backend::new(
                bucket, 
                region, 
                endpoint, 
                access_key_id, 
                secret_access_key,
                prefix,
            ).await?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio;

    #[tokio::test]
    async fn test_filesystem_backend() {
        use std::env;
        
        let temp_dir = env::temp_dir().join("minne_test_fs");
        let backend = filesystem::FilesystemBackend::new(temp_dir.to_string_lossy().to_string())
            .expect("Failed to create filesystem backend");

        let user_id = "test_user";
        let file_id = Uuid::new_v4();
        let filename = "test_file.txt";
        let content = Bytes::from("Hello, world!");

        // Test storing a file
        let storage_path = backend
            .store_file(user_id, &file_id, filename, content.clone(), Some("text/plain"))
            .await
            .expect("Failed to store file");

        // Test file exists
        assert!(backend.file_exists(&storage_path).await.expect("Failed to check file existence"));

        // Test retrieving the file
        let stored_file = backend
            .get_file(&storage_path)
            .await
            .expect("Failed to retrieve file");
        
        assert_eq!(stored_file.content, content);
        assert_eq!(stored_file.metadata.size, content.len() as u64);

        // Test getting metadata
        let metadata = backend
            .get_metadata(&storage_path)
            .await
            .expect("Failed to get metadata");
        
        assert_eq!(metadata.size, content.len() as u64);

        // Test deleting the file
        backend
            .delete_file(&storage_path)
            .await
            .expect("Failed to delete file");

        // Verify file no longer exists
        assert!(!backend.file_exists(&storage_path).await.expect("Failed to check file existence"));

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_storage_config_filesystem() {
        let config = StorageConfig::FileSystem {
            data_dir: "/tmp/test_storage".to_string(),
        };

        let backend = create_storage_backend(config).await.expect("Failed to create backend");
        
        // Test that we can use the backend
        let user_id = "test_user";
        let file_id = Uuid::new_v4();
        let filename = "config_test.txt";
        let content = Bytes::from("Config test content");

        let storage_path = backend
            .store_file(user_id, &file_id, filename, content, Some("text/plain"))
            .await
            .expect("Failed to store file");

        assert!(backend.file_exists(&storage_path).await.expect("Failed to check file existence"));

        // Cleanup
        let _ = backend.delete_file(&storage_path).await;
        let _ = tokio::fs::remove_dir_all("/tmp/test_storage").await;
    }

    #[cfg(feature = "integration-tests")]
    #[tokio::test]
    async fn test_s3_backend_integration() {
        // This test requires actual S3 credentials and should only run with integration tests
        use std::env;
        
        let bucket = env::var("TEST_S3_BUCKET").expect("TEST_S3_BUCKET env var required for S3 tests");
        let region = env::var("TEST_S3_REGION").ok();
        let endpoint = env::var("TEST_S3_ENDPOINT").ok();
        let access_key = env::var("TEST_S3_ACCESS_KEY").ok();
        let secret_key = env::var("TEST_S3_SECRET_KEY").ok();

        let backend = object_storage::S3Backend::new(
            bucket,
            region,
            endpoint,
            access_key,
            secret_key,
            Some("test".to_string()),
        ).await.expect("Failed to create S3 backend");

        let user_id = "test_user";
        let file_id = Uuid::new_v4();
        let filename = "s3_test.txt";
        let content = Bytes::from("S3 test content");

        // Test storing a file
        let storage_path = backend
            .store_file(user_id, &file_id, filename, content.clone(), Some("text/plain"))
            .await
            .expect("Failed to store file to S3");

        // Test file exists
        assert!(backend.file_exists(&storage_path).await.expect("Failed to check file existence"));

        // Test retrieving the file
        let stored_file = backend
            .get_file(&storage_path)
            .await
            .expect("Failed to retrieve file from S3");
        
        assert_eq!(stored_file.content, content);

        // Test getting metadata
        let metadata = backend
            .get_metadata(&storage_path)
            .await
            .expect("Failed to get metadata from S3");
        
        assert_eq!(metadata.size, content.len() as u64);

        // Test deleting the file
        backend
            .delete_file(&storage_path)
            .await
            .expect("Failed to delete file from S3");

        // Verify file no longer exists
        assert!(!backend.file_exists(&storage_path).await.expect("Failed to check file existence"));
    }
}