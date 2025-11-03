use axum_typed_multipart::FieldData;
use bytes;
use mime_guess::from_path;
use object_store::Error as ObjectStoreError;
use sha2::{Digest, Sha256};
use std::{
    io::{BufReader, Read},
    path::Path,
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

use crate::{
    error::AppError,
    storage::{db::SurrealDbClient, store, store::StorageManager},
    stored_object,
};

#[derive(Error, Debug)]
pub enum FileError {
    #[error("File not found for UUID: {0}")]
    FileNotFound(String),

    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("Duplicate file detected with SHA256: {0}")]
    DuplicateFile(String),

    #[error("SurrealDB error: {0}")]
    SurrealError(#[from] surrealdb::Error),

    #[error("Failed to persist file: {0}")]
    PersistError(#[from] tempfile::PersistError),

    #[error("File name missing in metadata")]
    MissingFileName,

    #[error("Object store error: {0}")]
    ObjectStore(#[from] ObjectStoreError),
}

stored_object!(FileInfo, "file", {
    sha256: String,
    path: String,
    file_name: String,
    mime_type: String,
    user_id: String
});

impl FileInfo {
    /// Guesses the MIME type based on the file extension.
    ///
    /// # Arguments
    /// * `path` - The path to the file.
    ///
    /// # Returns
    /// * `String` - The guessed MIME type as a string.
    fn guess_mime_type(path: &Path) -> String {
        from_path(path)
            .first_or(mime::APPLICATION_OCTET_STREAM)
            .to_string()
    }

    /// Calculates the SHA256 hash of the given file.
    ///
    /// # Arguments
    /// * `file` - The file to hash.
    ///
    /// # Returns
    /// * `Result<String, FileError>` - The SHA256 hash as a hex string or an error.
    async fn get_sha(file: &NamedTempFile) -> Result<String, FileError> {
        let mut reader = BufReader::new(file.as_file());
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192]; // 8KB buffer

        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        let digest = hasher.finalize();
        Ok(format!("{:x}", digest))
    }

    /// Sanitizes the file name to prevent security vulnerabilities like directory traversal.
    /// Replaces any non-alphanumeric characters (excluding '.' and '_') with underscores.
    fn sanitize_file_name(file_name: &str) -> String {
        if let Some(idx) = file_name.rfind('.') {
            let (name, ext) = file_name.split_at(idx);
            let sanitized_name: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("{}{}", sanitized_name, ext)
        } else {
            // No extension
            file_name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        }
    }

    /// Retrieves a `FileInfo` by SHA256.
    ///
    /// # Arguments
    /// * `sha256` - The SHA256 hash string.
    /// * `db_client` - Reference to the SurrealDbClient.
    ///
    /// # Returns
    /// * `Result<Option<FileInfo>, FileError>` - The `FileInfo` or `None` if not found.
    async fn get_by_sha(sha256: &str, db_client: &SurrealDbClient) -> Result<FileInfo, FileError> {
        let query = format!("SELECT * FROM file WHERE sha256 = '{}'", &sha256);
        let response: Vec<FileInfo> = db_client.client.query(query).await?.take(0)?;

        response
            .into_iter()
            .next()
            .ok_or(FileError::FileNotFound(sha256.to_string()))
    }

    /// Retrieves a `FileInfo` by its ID.
    ///
    /// # Arguments
    /// * `id` - The ID string of the file.
    /// * `db_client` - Reference to the SurrealDbClient.
    ///
    /// # Returns
    /// * `Result<FileInfo, FileError>` - The `FileInfo` or an error if not found or on DB issues.
    pub async fn get_by_id(id: &str, db_client: &SurrealDbClient) -> Result<FileInfo, FileError> {
        match db_client.get_item::<FileInfo>(id).await {
            Ok(Some(file_info)) => Ok(file_info),
            Ok(None) => Err(FileError::FileNotFound(id.to_string())),
            Err(e) => Err(FileError::SurrealError(e)),
        }
    }

    /// Create a new FileInfo using StorageManager for persistent storage operations.
    ///
    /// # Arguments
    /// * `field_data` - The uploaded file data
    /// * `db_client` - Reference to the SurrealDbClient
    /// * `user_id` - The user ID
    /// * `storage` - A StorageManager instance for storage operations
    ///
    /// # Returns
    /// * `Result<Self, FileError>` - The created FileInfo or an error
    pub async fn new_with_storage(
        field_data: FieldData<NamedTempFile>,
        db_client: &SurrealDbClient,
        user_id: &str,
        storage: &StorageManager,
    ) -> Result<Self, FileError> {
        let file = field_data.contents;
        let file_name = field_data
            .metadata
            .file_name
            .ok_or(FileError::MissingFileName)?;
        let original_file_name = file_name.clone();

        // Calculate SHA256
        let sha256 = Self::get_sha(&file).await?;

        // Early return if file already exists
        match Self::get_by_sha(&sha256, db_client).await {
            Ok(existing_file) => {
                info!("File already exists with SHA256: {}", sha256);
                return Ok(existing_file);
            }
            Err(FileError::FileNotFound(_)) => (), // Expected case for new files
            Err(e) => return Err(e),               // Propagate unexpected errors
        }

        // Generate UUID and prepare paths
        let uuid = Uuid::new_v4();
        let sanitized_file_name = Self::sanitize_file_name(&file_name);
        let now = Utc::now();

        let path =
            Self::persist_file_with_storage(&uuid, file, &sanitized_file_name, user_id, storage)
                .await?;

        // Create FileInfo struct
        let file_info = FileInfo {
            id: uuid.to_string(),
            user_id: user_id.to_string(),
            sha256,
            file_name: original_file_name,
            path,
            mime_type: Self::guess_mime_type(Path::new(&file_name)),
            created_at: now,
            updated_at: now,
        };

        // Store in database
        db_client
            .store_item(file_info.clone())
            .await
            .map_err(FileError::SurrealError)?;

        Ok(file_info)
    }

    /// Delete a FileInfo by ID using StorageManager for storage operations.
    ///
    /// # Arguments
    /// * `id` - ID of the FileInfo
    /// * `db_client` - Reference to SurrealDbClient
    /// * `storage` - A StorageManager instance for storage operations
    ///
    /// # Returns
    /// * `Result<(), AppError>` - Success or error
    pub async fn delete_by_id_with_storage(
        id: &str,
        db_client: &SurrealDbClient,
        storage: &StorageManager,
    ) -> Result<(), AppError> {
        // Get the FileInfo from the database
        let Some(file_info) = db_client.get_item::<FileInfo>(id).await? else {
            return Ok(());
        };

        // Remove the object's parent prefix in the object store
        let (parent_prefix, _file_name) = store::split_object_path(&file_info.path)
            .map_err(|e| AppError::from(anyhow::anyhow!(e)))?;
        storage
            .delete_prefix(&parent_prefix)
            .await
            .map_err(|e| AppError::from(anyhow::anyhow!(e)))?;
        info!(
            "Removed object prefix {} and its contents via StorageManager",
            parent_prefix
        );

        // Delete the FileInfo from the database
        db_client.delete_item::<FileInfo>(id).await?;

        Ok(())
    }

    /// Retrieve file content using StorageManager for storage operations.
    ///
    /// # Arguments
    /// * `storage` - A StorageManager instance for storage operations
    ///
    /// # Returns
    /// * `Result<bytes::Bytes, AppError>` - The file content or an error
    pub async fn get_content_with_storage(
        &self,
        storage: &StorageManager,
    ) -> Result<bytes::Bytes, AppError> {
        storage
            .get(&self.path)
            .await
            .map_err(|e: object_store::Error| AppError::from(anyhow::anyhow!(e)))
    }

    /// Persist file to storage using StorageManager.
    ///
    /// # Arguments
    /// * `uuid` - The UUID for the file
    /// * `file` - The temporary file to persist
    /// * `file_name` - The name of the file
    /// * `user_id` - The user ID
    /// * `storage` - A StorageManager instance for storage operations
    ///
    /// # Returns
    /// * `Result<String, FileError>` - The logical object location or an error.
    async fn persist_file_with_storage(
        uuid: &Uuid,
        file: NamedTempFile,
        file_name: &str,
        user_id: &str,
        storage: &StorageManager,
    ) -> Result<String, FileError> {
        // Logical object location relative to the store root
        let location = format!("{}/{}/{}", user_id, uuid, file_name);
        info!("Persisting to object location: {}", location);

        let bytes = tokio::fs::read(file.path()).await?;
        storage
            .put(&location, bytes.into())
            .await
            .map_err(FileError::from)?;

        Ok(location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::store::testing::TestStorageManager;
    use axum::http::HeaderMap;
    use axum_typed_multipart::FieldMetadata;
    use std::{io::Write, path::Path};
    use tempfile::NamedTempFile;

    /// Creates a test temporary file with the given content
    fn create_test_file(content: &[u8], file_name: &str) -> FieldData<NamedTempFile> {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
        temp_file
            .write_all(content)
            .expect("Failed to write to temp file");

        let metadata = FieldMetadata {
            name: Some("file".to_string()),
            file_name: Some(file_name.to_string()),
            content_type: None,
            headers: HeaderMap::default(),
        };

        let field_data = FieldData {
            metadata,
            contents: temp_file,
        };

        field_data
    }

    #[tokio::test]
    async fn test_fileinfo_create_read_delete_with_storage_manager() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.unwrap();

        let content = b"This is a test file for StorageManager operations";
        let file_name = "storage_manager_test.txt";
        let field_data = create_test_file(content, file_name);

        // Create test storage manager (memory backend)
        let test_storage = store::testing::TestStorageManager::new_memory()
            .await
            .expect("Failed to create test storage manager");

        // Create a FileInfo instance with storage manager
        let user_id = "test_user";

        // Test file creation with StorageManager
        let file_info =
            FileInfo::new_with_storage(field_data, &db, user_id, test_storage.storage())
                .await
                .expect("Failed to create file with StorageManager");
        assert_eq!(file_info.file_name, file_name);

        // Verify the file exists via StorageManager and has correct content
        let bytes = file_info
            .get_content_with_storage(test_storage.storage())
            .await
            .expect("Failed to read file content via StorageManager");
        assert_eq!(bytes.as_ref(), content);

        // Test file reading
        let retrieved = FileInfo::get_by_id(&file_info.id, &db)
            .await
            .expect("Failed to retrieve file info");
        assert_eq!(retrieved.id, file_info.id);
        assert_eq!(retrieved.sha256, file_info.sha256);
        assert_eq!(retrieved.file_name, file_name);

        // Test file deletion with StorageManager
        FileInfo::delete_by_id_with_storage(&file_info.id, &db, test_storage.storage())
            .await
            .expect("Failed to delete file with StorageManager");

        let deleted_result = file_info
            .get_content_with_storage(test_storage.storage())
            .await;
        assert!(deleted_result.is_err(), "File should be deleted");

        // No cleanup needed - TestStorageManager handles it automatically
    }

    #[tokio::test]
    async fn test_fileinfo_preserves_original_filename_and_sanitizes_path() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.unwrap();

        let content = b"filename sanitization";
        let original_name = "Complex name (1).txt";
        let expected_sanitized = "Complex_name__1_.txt";
        let field_data = create_test_file(content, original_name);

        let test_storage = store::testing::TestStorageManager::new_memory()
            .await
            .expect("Failed to create test storage manager");

        let file_info =
            FileInfo::new_with_storage(field_data, &db, "sanitized_user", test_storage.storage())
                .await
                .expect("Failed to create file via storage manager");

        assert_eq!(file_info.file_name, original_name);

        let stored_name = Path::new(&file_info.path)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("stored name");
        assert_eq!(stored_name, expected_sanitized);
    }

    #[tokio::test]
    async fn test_fileinfo_duplicate_detection_with_storage_manager() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations().await.unwrap();

        let content = b"This is a test file for StorageManager duplicate detection";
        let file_name = "storage_manager_duplicate.txt";
        let field_data = create_test_file(content, file_name);

        // Create test storage manager
        let test_storage = store::testing::TestStorageManager::new_memory()
            .await
            .expect("Failed to create test storage manager");

        // Create a FileInfo instance with storage manager
        let user_id = "test_user";

        // Store the original file
        let original_file_info =
            FileInfo::new_with_storage(field_data, &db, user_id, test_storage.storage())
                .await
                .expect("Failed to create original file with StorageManager");

        // Create another file with the same content but different name
        let duplicate_name = "storage_manager_duplicate_2.txt";
        let field_data2 = create_test_file(content, duplicate_name);

        // The system should detect it's the same file and return the original FileInfo
        let duplicate_file_info =
            FileInfo::new_with_storage(field_data2, &db, user_id, test_storage.storage())
                .await
                .expect("Failed to process duplicate file with StorageManager");

        // Verify duplicate detection worked
        assert_eq!(duplicate_file_info.id, original_file_info.id);
        assert_eq!(duplicate_file_info.sha256, original_file_info.sha256);
        assert_eq!(duplicate_file_info.file_name, file_name);
        assert_ne!(duplicate_file_info.file_name, duplicate_name);

        // Verify both files have the same content (they should point to the same file)
        let original_content = original_file_info
            .get_content_with_storage(test_storage.storage())
            .await
            .unwrap();
        let duplicate_content = duplicate_file_info
            .get_content_with_storage(test_storage.storage())
            .await
            .unwrap();
        assert_eq!(original_content.as_ref(), content);
        assert_eq!(duplicate_content.as_ref(), content);

        // Clean up
        FileInfo::delete_by_id_with_storage(&original_file_info.id, &db, test_storage.storage())
            .await
            .expect("Failed to delete original file with StorageManager");
    }

    #[tokio::test]
    async fn test_file_creation() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        let content = b"This is a test file content";
        let file_name = "test_file.txt";
        let field_data = create_test_file(content, file_name);

        // Create a FileInfo instance with StorageManager
        let user_id = "test_user";
        let test_storage = TestStorageManager::new_memory()
            .await
            .expect("create test storage manager");
        let file_info =
            FileInfo::new_with_storage(field_data, &db, user_id, test_storage.storage()).await;

        // Verify the FileInfo was created successfully
        assert!(file_info.is_ok());
        let file_info = file_info.unwrap();

        // Check essential properties
        assert!(!file_info.id.is_empty());
        assert_eq!(file_info.file_name, file_name);
        assert!(!file_info.sha256.is_empty());
        assert!(!file_info.path.is_empty());
        // path should be logical: "user_id/uuid/file_name"
        let parts: Vec<&str> = file_info.path.split('/').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], user_id);
        assert_eq!(parts[2], file_name);
        assert!(file_info.mime_type.contains("text/plain"));

        // Verify it's in the database
        let stored: Option<FileInfo> = db
            .get_item(&file_info.id)
            .await
            .expect("Failed to retrieve file info");
        assert!(stored.is_some());
        let stored = stored.unwrap();
        assert_eq!(stored.id, file_info.id);
        assert_eq!(stored.file_name, file_info.file_name);
        assert_eq!(stored.sha256, file_info.sha256);
    }

    #[tokio::test]
    async fn test_file_duplicate_detection() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        // First, store a file with known content
        let content = b"This is a test file for duplicate detection";
        let file_name = "original.txt";
        let user_id = "test_user";

        let test_storage = TestStorageManager::new_memory()
            .await
            .expect("create test storage manager");

        let field_data1 = create_test_file(content, file_name);
        let original_file_info =
            FileInfo::new_with_storage(field_data1, &db, user_id, test_storage.storage())
                .await
                .expect("Failed to create original file");

        // Now try to store another file with the same content but different name
        let duplicate_name = "duplicate.txt";
        let field_data2 = create_test_file(content, duplicate_name);

        // The system should detect it's the same file and return the original FileInfo
        let duplicate_file_info =
            FileInfo::new_with_storage(field_data2, &db, user_id, test_storage.storage())
                .await
                .expect("Failed to process duplicate file");

        // The returned FileInfo should match the original
        assert_eq!(duplicate_file_info.id, original_file_info.id);
        assert_eq!(duplicate_file_info.sha256, original_file_info.sha256);

        // But it should retain the original file name, not the duplicate's name
        assert_eq!(duplicate_file_info.file_name, file_name);
        assert_ne!(duplicate_file_info.file_name, duplicate_name);
    }

    #[tokio::test]
    async fn test_guess_mime_type() {
        // Test common file extensions
        assert_eq!(
            FileInfo::guess_mime_type(Path::new("test.txt")),
            "text/plain".to_string()
        );
        assert_eq!(
            FileInfo::guess_mime_type(Path::new("image.png")),
            "image/png".to_string()
        );
        assert_eq!(
            FileInfo::guess_mime_type(Path::new("document.pdf")),
            "application/pdf".to_string()
        );
        assert_eq!(
            FileInfo::guess_mime_type(Path::new("data.json")),
            "application/json".to_string()
        );

        // Test unknown extension
        assert_eq!(
            FileInfo::guess_mime_type(Path::new("unknown.929yz")),
            "application/octet-stream".to_string()
        );
    }

    #[tokio::test]
    async fn test_sanitize_file_name() {
        // Safe characters should remain unchanged
        assert_eq!(
            FileInfo::sanitize_file_name("normal_file.txt"),
            "normal_file.txt"
        );
        assert_eq!(FileInfo::sanitize_file_name("file123.doc"), "file123.doc");

        // Unsafe characters should be replaced with underscores
        assert_eq!(
            FileInfo::sanitize_file_name("file with spaces.txt"),
            "file_with_spaces.txt"
        );
        assert_eq!(
            FileInfo::sanitize_file_name("file/with/path.txt"),
            "file_with_path.txt"
        );
        assert_eq!(
            FileInfo::sanitize_file_name("file:with:colons.txt"),
            "file_with_colons.txt"
        );
        assert_eq!(
            FileInfo::sanitize_file_name("../dangerous.txt"),
            "___dangerous.txt"
        );
    }

    #[tokio::test]
    async fn test_get_by_sha_not_found() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Try to find a file with a SHA that doesn't exist
        let result = FileInfo::get_by_sha("nonexistent_sha_hash", &db).await;
        assert!(result.is_err());

        match result {
            Err(FileError::FileNotFound(_)) => {
                // Expected error
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_manual_file_info_creation() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a FileInfo instance directly
        let now = Utc::now();
        let file_info = FileInfo {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            user_id: "user123".to_string(),
            sha256: "test_sha256_hash".to_string(),
            path: "/path/to/file.txt".to_string(),
            file_name: "manual_file.txt".to_string(),
            mime_type: "text/plain".to_string(),
        };

        // Store it in the database
        let result = db.store_item(file_info.clone()).await;
        assert!(result.is_ok());

        // Verify it can be retrieved
        let retrieved: Option<FileInfo> = db
            .get_item(&file_info.id)
            .await
            .expect("Failed to retrieve file info");
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, file_info.id);
        assert_eq!(retrieved.sha256, file_info.sha256);
        assert_eq!(retrieved.file_name, file_info.file_name);
        assert_eq!(retrieved.path, file_info.path);
        assert_eq!(retrieved.mime_type, file_info.mime_type);
    }

    #[tokio::test]
    async fn test_delete_by_id() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");
        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        // Create and persist a test file via FileInfo::new_with_storage
        let user_id = "user123";
        let test_storage = TestStorageManager::new_memory()
            .await
            .expect("create test storage manager");
        let temp = create_test_file(b"test content", "test_file.txt");
        let file_info = FileInfo::new_with_storage(temp, &db, user_id, test_storage.storage())
            .await
            .expect("create file");

        // Delete the file using StorageManager
        let delete_result =
            FileInfo::delete_by_id_with_storage(&file_info.id, &db, test_storage.storage()).await;

        // Delete should be successful
        assert!(
            delete_result.is_ok(),
            "Failed to delete file: {:?}",
            delete_result
        );

        // Verify the file is removed from the database
        let retrieved: Option<FileInfo> = db
            .get_item(&file_info.id)
            .await
            .expect("Failed to query database");
        assert!(
            retrieved.is_none(),
            "FileInfo should be deleted from the database"
        );

        // Verify content no longer retrievable from storage
        assert!(test_storage.storage().get(&file_info.path).await.is_err());
    }

    #[tokio::test]
    async fn test_delete_by_id_not_found() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Try to delete a file that doesn't exist
        let test_storage = TestStorageManager::new_memory().await.unwrap();
        let result =
            FileInfo::delete_by_id_with_storage("nonexistent_id", &db, test_storage.storage())
                .await;

        // Should succeed even if the file record does not exist
        assert!(result.is_ok());
    }
    #[tokio::test]
    async fn test_get_by_id() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a FileInfo instance directly
        let now = Utc::now();
        let file_id = Uuid::new_v4().to_string();
        let original_file_info = FileInfo {
            id: file_id.clone(),
            user_id: "user123".to_string(),
            created_at: now,
            updated_at: now,
            sha256: "test_sha256_for_get_by_id".to_string(),
            path: "/path/to/get_by_id_test.txt".to_string(),
            file_name: "get_by_id_test.txt".to_string(),
            mime_type: "text/plain".to_string(),
        };

        // Store it in the database
        db.store_item(original_file_info.clone())
            .await
            .expect("Failed to store item for get_by_id test");

        // Retrieve it using get_by_id
        let result = FileInfo::get_by_id(&file_id, &db).await;

        // Assert success and content match
        assert!(result.is_ok());
        let retrieved_info = result.unwrap();
        assert_eq!(retrieved_info.id, original_file_info.id);
        assert_eq!(retrieved_info.sha256, original_file_info.sha256);
        assert_eq!(retrieved_info.file_name, original_file_info.file_name);
        assert_eq!(retrieved_info.path, original_file_info.path);
        assert_eq!(retrieved_info.mime_type, original_file_info.mime_type);
    }

    #[tokio::test]
    async fn test_get_by_id_not_found() {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Try to retrieve a non-existent ID
        let non_existent_id = "non-existent-file-id";
        let result = FileInfo::get_by_id(non_existent_id, &db).await;

        // Assert failure
        assert!(result.is_err());

        // Assert the specific error type is FileNotFound
        match result {
            Err(FileError::FileNotFound(id)) => {
                assert_eq!(id, non_existent_id);
            }
            Err(e) => panic!("Expected FileNotFound error, but got {:?}", e),
            Ok(_) => panic!("Expected an error, but got Ok"),
        }
    }

    // StorageManager-based tests
    #[tokio::test]
    async fn test_file_info_new_with_storage_memory() {
        // Setup
        let db = SurrealDbClient::memory("test_ns", "test_file_storage_memory")
            .await
            .unwrap();
        db.apply_migrations().await.unwrap();

        let content = b"This is a test file for StorageManager";
        let field_data = create_test_file(content, "test_storage.txt");
        let user_id = "test_user";

        // Create test storage manager
        let storage = store::testing::TestStorageManager::new_memory()
            .await
            .unwrap();

        // Test file creation with StorageManager
        let file_info = FileInfo::new_with_storage(field_data, &db, user_id, storage.storage())
            .await
            .expect("Failed to create file with StorageManager");

        // Verify the file was created correctly
        assert_eq!(file_info.user_id, user_id);
        assert_eq!(file_info.file_name, "test_storage.txt");
        assert!(!file_info.sha256.is_empty());
        assert!(!file_info.path.is_empty());

        // Test content retrieval with StorageManager
        let retrieved_content = file_info
            .get_content_with_storage(storage.storage())
            .await
            .expect("Failed to get file content with StorageManager");
        assert_eq!(retrieved_content.as_ref(), content);

        // Test file deletion with StorageManager
        FileInfo::delete_by_id_with_storage(&file_info.id, &db, storage.storage())
            .await
            .expect("Failed to delete file with StorageManager");

        // Verify file is deleted
        let deleted_content_result = file_info.get_content_with_storage(storage.storage()).await;
        assert!(deleted_content_result.is_err());
    }

    #[tokio::test]
    async fn test_file_info_new_with_storage_local() {
        // Setup
        let db = SurrealDbClient::memory("test_ns", "test_file_storage_local")
            .await
            .unwrap();
        db.apply_migrations().await.unwrap();

        let content = b"This is a test file for StorageManager with local storage";
        let field_data = create_test_file(content, "test_local.txt");
        let user_id = "test_user";

        // Create test storage manager with local backend
        let storage = store::testing::TestStorageManager::new_local()
            .await
            .unwrap();

        // Test file creation with StorageManager
        let file_info = FileInfo::new_with_storage(field_data, &db, user_id, storage.storage())
            .await
            .expect("Failed to create file with StorageManager");

        // Verify the file was created correctly
        assert_eq!(file_info.user_id, user_id);
        assert_eq!(file_info.file_name, "test_local.txt");
        assert!(!file_info.sha256.is_empty());
        assert!(!file_info.path.is_empty());

        // Test content retrieval with StorageManager
        let retrieved_content = file_info
            .get_content_with_storage(storage.storage())
            .await
            .expect("Failed to get file content with StorageManager");
        assert_eq!(retrieved_content.as_ref(), content);

        // Test file deletion with StorageManager
        FileInfo::delete_by_id_with_storage(&file_info.id, &db, storage.storage())
            .await
            .expect("Failed to delete file with StorageManager");

        // Verify file is deleted
        let deleted_content_result = file_info.get_content_with_storage(storage.storage()).await;
        assert!(deleted_content_result.is_err());
    }

    #[tokio::test]
    async fn test_file_info_storage_manager_persistence() {
        // Setup
        let db = SurrealDbClient::memory("test_ns", "test_file_persistence")
            .await
            .unwrap();
        db.apply_migrations().await.unwrap();

        let content = b"Test content for persistence";
        let field_data = create_test_file(content, "persistence_test.txt");
        let user_id = "test_user";

        // Create test storage manager
        let storage = store::testing::TestStorageManager::new_memory()
            .await
            .unwrap();

        // Create file
        let file_info = FileInfo::new_with_storage(field_data, &db, user_id, storage.storage())
            .await
            .expect("Failed to create file");

        // Test that data persists across multiple operations with the same StorageManager
        let retrieved_content_1 = file_info
            .get_content_with_storage(storage.storage())
            .await
            .unwrap();
        let retrieved_content_2 = file_info
            .get_content_with_storage(storage.storage())
            .await
            .unwrap();

        assert_eq!(retrieved_content_1.as_ref(), content);
        assert_eq!(retrieved_content_2.as_ref(), content);

        // Test that different StorageManager instances don't share data (memory storage isolation)
        let storage2 = store::testing::TestStorageManager::new_memory()
            .await
            .unwrap();
        let isolated_content_result = file_info.get_content_with_storage(storage2.storage()).await;
        assert!(
            isolated_content_result.is_err(),
            "Different StorageManager should not have access to same data"
        );
    }

    #[tokio::test]
    async fn test_file_info_storage_manager_equivalence() {
        // Setup
        let db = SurrealDbClient::memory("test_ns", "test_file_equivalence")
            .await
            .unwrap();
        db.apply_migrations().await.unwrap();

        let content = b"Test content for equivalence testing";
        let field_data1 = create_test_file(content, "equivalence_test_1.txt");
        let field_data2 = create_test_file(content, "equivalence_test_2.txt");
        let user_id = "test_user";

        // Create single storage manager and reuse it
        let storage_manager = store::testing::TestStorageManager::new_memory()
            .await
            .unwrap();
        let storage = storage_manager.storage();

        // Create multiple files with the same storage manager
        let file_info_1 = FileInfo::new_with_storage(field_data1, &db, user_id, &storage)
            .await
            .expect("Failed to create file 1");

        let file_info_2 = FileInfo::new_with_storage(field_data2, &db, user_id, &storage)
            .await
            .expect("Failed to create file 2");

        // Test that both files can be retrieved with the same storage backend
        let content_1 = file_info_1
            .get_content_with_storage(&storage)
            .await
            .unwrap();
        let content_2 = file_info_2
            .get_content_with_storage(&storage)
            .await
            .unwrap();

        assert_eq!(content_1.as_ref(), content);
        assert_eq!(content_2.as_ref(), content);

        // Test that files can be deleted with the same storage manager
        FileInfo::delete_by_id_with_storage(&file_info_1.id, &db, &storage)
            .await
            .unwrap();
        FileInfo::delete_by_id_with_storage(&file_info_2.id, &db, &storage)
            .await
            .unwrap();

        // Verify files are deleted
        let deleted_content_1 = file_info_1.get_content_with_storage(&storage).await;
        let deleted_content_2 = file_info_2.get_content_with_storage(&storage).await;

        assert!(deleted_content_1.is_err());
        assert!(deleted_content_2.is_err());
    }
}
