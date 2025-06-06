use axum_typed_multipart::FieldData;
use mime_guess::from_path;
use sha2::{Digest, Sha256};
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::fs::remove_dir_all;
use tracing::info;
use uuid::Uuid;

use crate::{
    error::AppError, storage::db::SurrealDbClient, stored_object, utils::config::AppConfig,
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
}

stored_object!(FileInfo, "file", {
    sha256: String,
    path: String,
    file_name: String,
    mime_type: String,
    user_id: String
});

impl FileInfo {
    pub async fn new(
        field_data: FieldData<NamedTempFile>,
        db_client: &SurrealDbClient,
        user_id: &str,
        config: &AppConfig,
    ) -> Result<Self, FileError> {
        info!("Data_dir: {:?}", config);
        let file = field_data.contents;
        let file_name = field_data
            .metadata
            .file_name
            .ok_or(FileError::MissingFileName)?;

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
        // Create new FileInfo instance
        let file_info = Self {
            id: uuid.to_string(),
            created_at: now,
            updated_at: now,
            file_name,
            sha256,
            path: Self::persist_file(&uuid, file, &sanitized_file_name, user_id, config)
                .await?
                .to_string_lossy()
                .into(),
            mime_type: Self::guess_mime_type(Path::new(&sanitized_file_name)),
            user_id: user_id.to_string(),
        };

        // Store in database
        db_client.store_item(file_info.clone()).await?;

        Ok(file_info)
    }

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

    /// Persists the file to the filesystem under `{data_dir}/{user_id}/{uuid}/{file_name}`.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file.
    /// * `file` - The temporary file to persist.
    /// * `file_name` - The sanitized file name.
    /// * `user-id` - User id
    /// * `config` - Application configuration containing data directory path
    ///
    /// # Returns
    /// * `Result<PathBuf, FileError>` - The persisted file path or an error.
    async fn persist_file(
        uuid: &Uuid,
        file: NamedTempFile,
        file_name: &str,
        user_id: &str,
        config: &AppConfig,
    ) -> Result<PathBuf, FileError> {
        info!("Data dir: {:?}", config.data_dir);
        // Convert relative path to absolute path
        let base_dir = if config.data_dir.starts_with('/') {
            Path::new(&config.data_dir).to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(FileError::Io)?
                .join(&config.data_dir)
        };

        let user_dir = base_dir.join(user_id); // Create the user directory
        let uuid_dir = user_dir.join(uuid.to_string()); // Create the UUID directory under the user directory

        // Create the user and UUID directories if they don't exist
        tokio::fs::create_dir_all(&uuid_dir)
            .await
            .map_err(FileError::Io)?;

        // Define the final file path
        let final_path = uuid_dir.join(file_name);
        info!("Final path: {:?}", final_path);

        // Copy the temporary file to the final path
        tokio::fs::copy(file.path(), &final_path)
            .await
            .map_err(FileError::Io)?;
        info!("Copied file to {:?}", final_path);

        Ok(final_path)
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

    /// Removes FileInfo from database and file from disk
    ///
    /// # Arguments
    /// * `id` - Id of the FileInfo
    /// * `db_client` - Reference to SurrealDbClient
    ///
    /// # Returns
    ///  `Result<(), FileError>`
    pub async fn delete_by_id(id: &str, db_client: &SurrealDbClient) -> Result<(), AppError> {
        // Get the FileInfo from the database
        let file_info = match db_client.get_item::<FileInfo>(id).await? {
            Some(info) => info,
            None => {
                return Err(AppError::from(FileError::FileNotFound(format!(
                    "File with id {} was not found",
                    id
                ))))
            }
        };

        // Remove the file and its parent directory
        let file_path = Path::new(&file_info.path);
        if file_path.exists() {
            // Get the parent directory of the file
            if let Some(parent_dir) = file_path.parent() {
                // Remove the entire directory containing the file
                remove_dir_all(parent_dir).await?;
                info!("Removed directory {:?} and its contents", parent_dir);
            } else {
                return Err(AppError::from(FileError::FileNotFound(
                    "File has no parent directory".to_string(),
                )));
            }
        } else {
            return Err(AppError::from(FileError::FileNotFound(format!(
                "File at path {:?} was not found",
                file_path
            ))));
        }

        // Delete the FileInfo from the database
        db_client.delete_item::<FileInfo>(id).await?;

        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use axum_typed_multipart::FieldMetadata;
    use std::io::Write;
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
    async fn test_cross_filesystem_file_operations() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a test file
        let content = b"This is a test file for cross-filesystem operations";
        let file_name = "cross_fs_test.txt";
        let field_data = create_test_file(content, file_name);

        // Create a FileInfo instance with data_dir in /tmp
        let user_id = "test_user";
        let config = AppConfig {
            data_dir: "/tmp/minne_test_data".to_string(), // Using /tmp which is typically on a different filesystem
            openai_api_key: "test_key".to_string(),
            surrealdb_address: "test_address".to_string(),
            surrealdb_username: "test_user".to_string(),
            surrealdb_password: "test_pass".to_string(),
            surrealdb_namespace: "test_ns".to_string(),
            surrealdb_database: "test_db".to_string(),
            http_port: 3000,
            openai_base_url: "..".to_string(),
        };

        // Test file creation
        let file_info = FileInfo::new(field_data, &db, user_id, &config)
            .await
            .expect("Failed to create file across filesystems");

        // Verify the file exists and has correct content
        let file_path = Path::new(&file_info.path);
        assert!(file_path.exists(), "File should exist at {:?}", file_path);

        let file_content = tokio::fs::read_to_string(file_path)
            .await
            .expect("Failed to read file content");
        assert_eq!(file_content, String::from_utf8_lossy(content));

        // Test file reading
        let retrieved = FileInfo::get_by_id(&file_info.id, &db)
            .await
            .expect("Failed to retrieve file info");
        assert_eq!(retrieved.id, file_info.id);
        assert_eq!(retrieved.sha256, file_info.sha256);

        // Test file deletion
        FileInfo::delete_by_id(&file_info.id, &db)
            .await
            .expect("Failed to delete file");
        assert!(!file_path.exists(), "File should be deleted");

        // Clean up the test directory
        let _ = tokio::fs::remove_dir_all(&config.data_dir).await;
    }

    #[tokio::test]
    async fn test_cross_filesystem_duplicate_detection() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a test file
        let content = b"This is a test file for cross-filesystem duplicate detection";
        let file_name = "cross_fs_duplicate.txt";
        let field_data = create_test_file(content, file_name);

        // Create a FileInfo instance with data_dir in /tmp
        let user_id = "test_user";
        let config = AppConfig {
            data_dir: "/tmp/minne_test_data".to_string(),
            openai_api_key: "test_key".to_string(),
            surrealdb_address: "test_address".to_string(),
            surrealdb_username: "test_user".to_string(),
            surrealdb_password: "test_pass".to_string(),
            surrealdb_namespace: "test_ns".to_string(),
            surrealdb_database: "test_db".to_string(),
            http_port: 3000,
            openai_base_url: "..".to_string(),
        };

        // Store the original file
        let original_file_info = FileInfo::new(field_data, &db, user_id, &config)
            .await
            .expect("Failed to create original file");

        // Create another file with the same content but different name
        let duplicate_name = "cross_fs_duplicate_2.txt";
        let field_data2 = create_test_file(content, duplicate_name);

        // The system should detect it's the same file and return the original FileInfo
        let duplicate_file_info = FileInfo::new(field_data2, &db, user_id, &config)
            .await
            .expect("Failed to process duplicate file");

        // Verify duplicate detection worked
        assert_eq!(duplicate_file_info.id, original_file_info.id);
        assert_eq!(duplicate_file_info.sha256, original_file_info.sha256);
        assert_eq!(duplicate_file_info.file_name, file_name);
        assert_ne!(duplicate_file_info.file_name, duplicate_name);

        // Clean up
        FileInfo::delete_by_id(&original_file_info.id, &db)
            .await
            .expect("Failed to delete file");
        let _ = tokio::fs::remove_dir_all(&config.data_dir).await;
    }

    #[tokio::test]
    async fn test_file_creation() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a test file
        let content = b"This is a test file content";
        let file_name = "test_file.txt";
        let field_data = create_test_file(content, file_name);

        // Create a FileInfo instance
        let user_id = "test_user";
        let config = AppConfig {
            data_dir: "./data".to_string(),
            openai_api_key: "test_key".to_string(),
            surrealdb_address: "test_address".to_string(),
            surrealdb_username: "test_user".to_string(),
            surrealdb_password: "test_pass".to_string(),
            surrealdb_namespace: "test_ns".to_string(),
            surrealdb_database: "test_db".to_string(),
            http_port: 3000,
            openai_base_url: "..".to_string(),
        };
        let file_info = FileInfo::new(field_data, &db, user_id, &config).await;

        // We can't fully test persistence to disk in unit tests,
        // but we can verify the database record was created
        assert!(file_info.is_ok());
        let file_info = file_info.unwrap();

        // Check essential properties
        assert!(!file_info.id.is_empty());
        assert_eq!(file_info.file_name, file_name);
        assert!(!file_info.sha256.is_empty());
        assert!(!file_info.path.is_empty());
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
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // First, store a file with known content
        let content = b"This is a test file for duplicate detection";
        let file_name = "original.txt";
        let user_id = "test_user";

        let config = AppConfig {
            data_dir: "./data".to_string(),
            openai_api_key: "test_key".to_string(),
            surrealdb_address: "test_address".to_string(),
            surrealdb_username: "test_user".to_string(),
            surrealdb_password: "test_pass".to_string(),
            surrealdb_namespace: "test_ns".to_string(),
            surrealdb_database: "test_db".to_string(),
            http_port: 3000,
            openai_base_url: "..".to_string(),
        };

        let field_data1 = create_test_file(content, file_name);
        let original_file_info = FileInfo::new(field_data1, &db, user_id, &config)
            .await
            .expect("Failed to create original file");

        // Now try to store another file with the same content but different name
        let duplicate_name = "duplicate.txt";
        let field_data2 = create_test_file(content, duplicate_name);

        // The system should detect it's the same file and return the original FileInfo
        let duplicate_file_info = FileInfo::new(field_data2, &db, user_id, &config)
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
        // Setup in-memory database for testing
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
        // Setup in-memory database for testing
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
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a FileInfo instance directly (without persistence to disk)
        let now = Utc::now();
        let file_id = Uuid::new_v4().to_string();

        // Create a temporary directory that mimics the structure we would have on disk
        let base_dir = Path::new("./data");
        let user_id = "test_user";
        let user_dir = base_dir.join(user_id);
        let uuid_dir = user_dir.join(&file_id);

        tokio::fs::create_dir_all(&uuid_dir)
            .await
            .expect("Failed to create test directories");

        // Create a test file in the directory
        let test_file_path = uuid_dir.join("test_file.txt");
        tokio::fs::write(&test_file_path, b"test content")
            .await
            .expect("Failed to write test file");

        // The file path should point to our test file
        let file_info = FileInfo {
            id: file_id.clone(),
            user_id: "user123".to_string(),
            created_at: now,
            updated_at: now,
            sha256: "test_sha256_hash".to_string(),
            path: test_file_path.to_string_lossy().to_string(),
            file_name: "test_file.txt".to_string(),
            mime_type: "text/plain".to_string(),
        };

        // Store it in the database
        db.store_item(file_info.clone())
            .await
            .expect("Failed to store file info");

        // Verify file exists on disk
        assert!(tokio::fs::try_exists(&test_file_path)
            .await
            .unwrap_or(false));

        // Delete the file
        let delete_result = FileInfo::delete_by_id(&file_id, &db).await;

        // Delete should be successful
        assert!(
            delete_result.is_ok(),
            "Failed to delete file: {:?}",
            delete_result
        );

        // Verify the file is removed from the database
        let retrieved: Option<FileInfo> = db
            .get_item(&file_id)
            .await
            .expect("Failed to query database");
        assert!(
            retrieved.is_none(),
            "FileInfo should be deleted from the database"
        );

        // Verify directory is gone
        assert!(
            !tokio::fs::try_exists(&uuid_dir).await.unwrap_or(true),
            "UUID directory should be deleted"
        );

        // Clean up test directory if it exists
        let _ = tokio::fs::remove_dir_all(base_dir).await;
    }

    #[tokio::test]
    async fn test_delete_by_id_not_found() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Try to delete a file that doesn't exist
        let result = FileInfo::delete_by_id("nonexistent_id", &db).await;

        // Should fail with FileNotFound error
        assert!(result.is_err());
        match result {
            Err(AppError::File(_)) => {
                // Expected error
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }
    #[tokio::test]
    async fn test_get_by_id() {
        // Setup in-memory database for testing
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
        // Setup in-memory database for testing
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

    #[tokio::test]
    async fn test_data_directory_configuration() {
        // Setup in-memory database for testing
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        // Create a test file
        let content = b"This is a test file for data directory configuration";
        let file_name = "data_dir_test.txt";
        let field_data = create_test_file(content, file_name);

        // Create a FileInfo instance with a custom data directory
        let user_id = "test_user";
        let custom_data_dir = "/tmp/minne_custom_data_dir";
        let config = AppConfig {
            data_dir: custom_data_dir.to_string(),
            openai_api_key: "test_key".to_string(),
            surrealdb_address: "test_address".to_string(),
            surrealdb_username: "test_user".to_string(),
            surrealdb_password: "test_pass".to_string(),
            surrealdb_namespace: "test_ns".to_string(),
            surrealdb_database: "test_db".to_string(),
            http_port: 3000,
            openai_base_url: "..".to_string(),
        };

        // Test file creation
        let file_info = FileInfo::new(field_data, &db, user_id, &config)
            .await
            .expect("Failed to create file in custom data directory");

        // Verify the file exists in the correct location
        let file_path = Path::new(&file_info.path);
        assert!(file_path.exists(), "File should exist at {:?}", file_path);

        // Verify the file is in the correct data directory
        assert!(
            file_path.starts_with(custom_data_dir),
            "File should be stored in the custom data directory"
        );

        // Verify the file has the correct content
        let file_content = tokio::fs::read_to_string(file_path)
            .await
            .expect("Failed to read file content");
        assert_eq!(file_content, String::from_utf8_lossy(content));

        // Test file deletion
        FileInfo::delete_by_id(&file_info.id, &db)
            .await
            .expect("Failed to delete file");
        assert!(!file_path.exists(), "File should be deleted");

        // Clean up the test directory
        let _ = tokio::fs::remove_dir_all(custom_data_dir).await;
    }
}
