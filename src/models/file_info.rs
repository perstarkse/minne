use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use axum_typed_multipart::FieldData;
use mime_guess::from_path;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use surrealdb::RecordId;
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info};
use uuid::Uuid;

use crate::{redis::client::{RedisClient, RedisClientTrait}, surrealdb::SurrealDbClient};

#[derive(Debug, Deserialize)]
struct Record {
    id: RecordId,
}

/// Represents metadata and storage information for a file.
#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    pub uuid: String,
    pub sha256: String,
    pub path: String,
    pub mime_type: String,
}

/// Errors that can occur during FileInfo operations
#[derive(Error, Debug)]
pub enum FileError {
    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("MIME type detection failed for input: {0}")]
    MimeDetection(String),

    #[error("Unsupported MIME type: {0}")]
    UnsupportedMime(String),

    #[error("SurrealDB error: {0}")]
    SurrealError(#[from] surrealdb::Error),

    #[error("Redis error: {0}")]
    RedisError(#[from] crate::redis::client::RedisError),

    #[error("File not found for UUID: {0}")]
    FileNotFound(String),

    #[error("Duplicate file detected with SHA256: {0}")]
    DuplicateFile(String),

    #[error("Hash collision detected")]
    HashCollision,

    #[error("Invalid UUID format: {0}")]
    InvalidUuid(String),

    #[error("File name missing in metadata")]
    MissingFileName,

    #[error("Failed to persist file: {0}")]
    PersistError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    // Add more error variants as needed.
}

impl IntoResponse for FileError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            FileError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
            FileError::Utf8(_) => (StatusCode::BAD_REQUEST, "Invalid UTF-8 data"),
            FileError::MimeDetection(_) => (StatusCode::BAD_REQUEST, "MIME type detection failed"),
            FileError::UnsupportedMime(_) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Unsupported MIME type"),
            FileError::RedisError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Redis error"),
            FileError::FileNotFound(_) => (StatusCode::NOT_FOUND, "File not found"),
            FileError::DuplicateFile(_) => (StatusCode::CONFLICT, "Duplicate file detected"),
            FileError::HashCollision => (StatusCode::INTERNAL_SERVER_ERROR, "Hash collision detected"),
            FileError::InvalidUuid(_) => (StatusCode::BAD_REQUEST, "Invalid UUID format"),
            FileError::MissingFileName => (StatusCode::BAD_REQUEST, "Missing file name in metadata"),
            FileError::PersistError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to persist file"),
            FileError::SerializationError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Serialization error"),
            FileError::DeserializationError(_) => (StatusCode::BAD_REQUEST, "Deserialization error"),
            FileError::SurrealError(_) =>(StatusCode::INTERNAL_SERVER_ERROR, "Serialization error"), 
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

impl FileInfo {
    pub async fn new(
        field_data: FieldData<NamedTempFile>,
        db_client: &SurrealDbClient,
    ) -> Result<FileInfo, FileError> {
        let file = field_data.contents; // NamedTempFile
        let metadata = field_data.metadata;

        // Extract file name from metadata
        let file_name = metadata.file_name.ok_or(FileError::MissingFileName)?;
        info!("File name: {:?}", file_name);

        // Calculate SHA256 hash of the file
        let sha = Self::get_sha(&file).await?;
        info!("SHA256: {:?}", sha);

        // Check if SHA exists in SurrealDB
        if let Some(existing_file_info) = Self::get_by_sha(&sha, db_client).await? {
            info!("Duplicate file detected with SHA256: {}", sha);
            return Ok(existing_file_info);
        }

        // Generate a new UUID
        let uuid = Uuid::new_v4();
        info!("UUID: {:?}", uuid);

        // Sanitize file name
        let sanitized_file_name = sanitize_file_name(&file_name);
        info!("Sanitized file name: {:?}", sanitized_file_name);

        // Persist the file to the filesystem
        let persisted_path = Self::persist_file(&uuid, file, &sanitized_file_name).await?;

        // Guess the MIME type
        let mime_type = Self::guess_mime_type(&persisted_path);
        info!("Mime type: {:?}", mime_type);

        // Construct the FileInfo object
        let file_info = FileInfo {
            uuid: uuid.to_string(),
            sha256: sha.clone(),
            path: persisted_path.to_string_lossy().to_string(),
            mime_type,
        };

        // Store FileInfo in SurrealDB
        Self::create_record(&file_info, db_client).await?;

        Ok(file_info)
    }

    /// Updates an existing file identified by UUID with new file data.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file to update.
    /// * `new_field_data` - The new file data.
    /// * `redis_client` - Reference to the RedisClient.
    ///
    /// # Returns
    /// * `Result<FileInfo, FileError>` - The updated `FileInfo` or an error.
    pub async fn update(uuid: Uuid, new_field_data: FieldData<NamedTempFile>, db_client: &SurrealDbClient) -> Result<FileInfo, FileError> {
        let new_file = new_field_data.contents;
        let new_metadata = new_field_data.metadata;

        // Extract new file name
        let new_file_name = new_metadata.file_name.ok_or(FileError::MissingFileName)?;

        // Calculate SHA256 of the new file
        let new_sha = Self::get_sha(&new_file).await?;

        // Check if the new SHA already exists
        if let Some(existing_file_info) = Self::get_by_sha(&new_sha, &db_client).await? {
            info!("Duplicate file detected with SHA256: {}", new_sha);
            return Ok(existing_file_info);
        }

        // Sanitize new file name
        let sanitized_new_file_name = sanitize_file_name(&new_file_name);

        // Persist the new file
        let new_persisted_path = Self::persist_file(&uuid, new_file, &sanitized_new_file_name).await?;

        // Guess the new MIME type
        let new_mime_type = Self::guess_mime_type(&new_persisted_path);

        // Get the existing item and remove it
        let old_record = Self::get_by_uuid(uuid, &db_client).await?;
        Self::delete_record(&old_record.sha256, &db_client).await?;

        // Update FileInfo
        let updated_file_info = FileInfo {
            uuid: uuid.to_string(),
            sha256: new_sha.clone(),
            path: new_persisted_path.to_string_lossy().to_string(),
            mime_type: new_mime_type,
        };

        // Save the new item
        Self::create_record(&updated_file_info,&db_client).await?;

        // Optionally, delete the old file from the filesystem if it's no longer referenced
        // This requires reference counting or checking if other FileInfo entries point to the same SHA
        // For simplicity, this step is omitted.

        Ok(updated_file_info)
    }

    /// Deletes a file and its corresponding metadata based on UUID.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file to delete.
    /// * `redis_client` - Reference to the RedisClient.
    ///
    /// # Returns
    /// * `Result<(), FileError>` - Empty result or an error.
    pub async fn delete(uuid: Uuid, db_client: &SurrealDbClient) -> Result<(), FileError> {
        // Retrieve FileInfo to get SHA256 and path
        let file_info = Self::get_by_uuid(uuid, &db_client).await?; 

        // Delete the file from the filesystem
        let file_path = Path::new(&file_info.path);
        if file_path.exists() {
            tokio::fs::remove_file(file_path).await.map_err(FileError::Io)?;
            info!("Deleted file at path: {}", file_info.path);
        } else {
            info!("File path does not exist, skipping deletion: {}", file_info.path);
        }

        // Delete the FileInfo from database
        Self::delete_record(&file_info.sha256, &db_client).await?;

        // Remove the UUID directory if empty
        let uuid_dir = file_path.parent().ok_or(FileError::FileNotFound(uuid.to_string()))?;
        if uuid_dir.exists() {
            let mut entries = tokio::fs::read_dir(uuid_dir).await.map_err(FileError::Io)?;
            if entries.next_entry().await?.is_none() {
                tokio::fs::remove_dir(uuid_dir).await.map_err(FileError::Io)?;
                info!("Deleted empty UUID directory: {:?}", uuid_dir);
            }
        }

        Ok(())
    }

    /// Persists the file to the filesystem under `./data/{uuid}/{file_name}`.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file.
    /// * `file` - The temporary file to persist.
    /// * `file_name` - The sanitized file name.
    ///
    /// # Returns
    /// * `Result<PathBuf, FileError>` - The persisted file path or an error.
    async fn persist_file(uuid: &Uuid, file: NamedTempFile, file_name: &str) -> Result<PathBuf, FileError> {
        let base_dir = Path::new("./data");
        let uuid_dir = base_dir.join(uuid.to_string());

        // Create the UUID directory if it doesn't exist
        tokio::fs::create_dir_all(&uuid_dir).await.map_err(FileError::Io)?;

        // Define the final file path
        let final_path = uuid_dir.join(file_name);
        info!("Final path: {:?}", final_path);

        // Persist the temporary file to the final path
        file.persist(&final_path).map_err(|e| FileError::PersistError(e.to_string()))?;

        info!("Persisted file to {:?}", final_path);

        Ok(final_path)
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

    
    /// Creates a new record in SurrealDB for the given `FileInfo`.
    ///
    /// # Arguments
    /// * `file_info` - The `FileInfo` to store.
    /// * `db_client` - Reference to the SurrealDbClient.
    ///
    /// # Returns
    /// * `Result<(), FileError>` - Empty result or an error.
    async fn create_record(file_info: &FileInfo, db_client: &SurrealDbClient) -> Result<(), FileError> {
        // Define the table and primary key

        // Create the record
        let _created: Option<Record> = db_client
            .client
            .create(("file", &file_info.uuid ))
            .content(file_info.clone())
            .await?;

        debug!("{:?}",_created);
                
        info!("Created FileInfo record with SHA256: {}", file_info.sha256);

        Ok(())
    }

    /// Retrieves a `FileInfo` by UUID.
    ///
    /// # Arguments
    /// * `uuid` - The UUID string.
    /// * `db_client` - Reference to the SurrealDbClient.
    ///
    /// # Returns
    /// * `Result<FileInfo, FileError>` - The `FileInfo` or `Error` if not found.
    pub async fn get_by_uuid(uuid: Uuid, db_client: &SurrealDbClient) -> Result<FileInfo, FileError> {
        let query = format!("SELECT * FROM file WHERE uuid = '{}'", uuid);
        let response: Vec<FileInfo> = db_client.client.query(query).await?.take(0)?;

        Ok(response.into_iter().next().ok_or(FileError::FileNotFound(uuid.to_string()))?)
    }

    /// Retrieves a `FileInfo` by SHA256.
    ///
    /// # Arguments
    /// * `sha256` - The SHA256 hash string.
    /// * `db_client` - Reference to the SurrealDbClient.
    ///
    /// # Returns
    /// * `Result<Option<FileInfo>, FileError>` - The `FileInfo` or `None` if not found.
    async fn get_by_sha(sha256: &str, db_client: &SurrealDbClient) -> Result<Option<FileInfo>, FileError> {
        let query = format!("SELECT * FROM file WHERE sha256 = '{}'", sha256);
        let response: Vec<FileInfo> = db_client.client.query(query).await?.take(0)?;

        debug!("{:?}", response);

        Ok(response.into_iter().next())
    }

    /// Deletes a `FileInfo` record by SHA256.
    ///
    /// # Arguments
    /// * `sha256` - The SHA256 hash string.
    /// * `db_client` - Reference to the SurrealDbClient.
    ///
    /// # Returns
    /// * `Result<(), FileError>` - Empty result or an error.
    async fn delete_record(sha256: &str, db_client: &SurrealDbClient) -> Result<(), FileError> {
        let table = "file";
        let primary_key = sha256;

        let _created: Option<Record> = db_client
            .client
            .delete((table, primary_key))
            .await?;

        info!("Deleted FileInfo record with SHA256: {}", sha256);

        Ok(())
    }
}

/// Sanitizes the file name to prevent security vulnerabilities like directory traversal.
/// Replaces any non-alphanumeric characters (excluding '.' and '_') with underscores.
fn sanitize_file_name(file_name: &str) -> String {
    file_name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' })
        .collect()
}
