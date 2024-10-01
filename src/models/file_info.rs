use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use axum_typed_multipart::FieldData;
use mime_guess::from_path;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

use crate::redis::client::RedisClient;

/// Represents metadata and storage information for a file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    pub uuid: Uuid,
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

    #[error("Redis error: {0}")]
    RedisError(#[from] crate::redis::client::RedisError),

    #[error("File not found for UUID: {0}")]
    FileNotFound(Uuid),

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
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

impl FileInfo {
    /// Creates a new `FileInfo` instance from uploaded field data.
    ///
    /// # Arguments
    /// * `field_data` - The uploaded file data.
    ///
    /// # Returns
    /// * `Result<FileInfo, FileError>` - The created `FileInfo` or an error.
    pub async fn new(field_data: FieldData<NamedTempFile>, redis_client: &RedisClient) -> Result<FileInfo, FileError> {
        let file = field_data.contents; // NamedTempFile
        let metadata = field_data.metadata;

        // Extract file name from metadata
        let file_name = metadata.file_name.ok_or(FileError::MissingFileName)?;
        info!("File name: {:?}", file_name);

        // Calculate SHA256 hash of the file
        let sha = Self::get_sha(&file).await?;
        info!("SHA256: {:?}", sha);

        // Check if SHA exists in Redis
        if let Some(existing_file_info) = redis_client.get_file_info_by_sha(&sha).await? {
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
            uuid,
            sha256: sha.clone(),
            path: persisted_path.to_string_lossy().to_string(),
            mime_type,
        };

        // Store FileInfo in Redis with SHA256 as key
        redis_client.set_file_info(&sha, &file_info).await?;

        // Map UUID to SHA256 in Redis
        redis_client.set_sha_uuid_mapping(&uuid, &sha).await?;

        Ok(file_info)
    }

    /// Retrieves `FileInfo` based on UUID.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file.
    ///
    /// # Returns
    /// * `Result<FileInfo, FileError>` - The `FileInfo` or an error.
    pub async fn get(uuid: Uuid, redis_client: &RedisClient) -> Result<FileInfo, FileError> {
        // Fetch SHA256 from UUID mapping
        let sha = redis_client.get_sha_by_uuid(&uuid).await?
            .ok_or(FileError::FileNotFound(uuid))?;

        // Retrieve FileInfo by SHA256
        let file_info = redis_client.get_file_info_by_sha(&sha).await?
            .ok_or(FileError::FileNotFound(uuid))?;

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
    pub async fn update(uuid: Uuid, new_field_data: FieldData<NamedTempFile>, redis_client: &RedisClient) -> Result<FileInfo, FileError> {
        let new_file = new_field_data.contents;
        let new_metadata = new_field_data.metadata;

        // Extract new file name
        let new_file_name = new_metadata.file_name.ok_or(FileError::MissingFileName)?;

        // Calculate SHA256 of the new file
        let new_sha = Self::get_sha(&new_file).await?;

        // Check if the new SHA already exists
        if let Some(existing_file_info) = redis_client.get_file_info_by_sha(&new_sha).await? {
            info!("Duplicate file detected with SHA256: {}", new_sha);
            return Ok(existing_file_info);
        }

        // Sanitize new file name
        let sanitized_new_file_name = sanitize_file_name(&new_file_name);

        // Persist the new file
        let new_persisted_path = Self::persist_file(&uuid, new_file, &sanitized_new_file_name).await?;

        // Guess the new MIME type
        let new_mime_type = Self::guess_mime_type(&new_persisted_path);

        // Retrieve existing FileInfo to get old SHA
        let old_file_info = Self::get(uuid, redis_client).await?;

        // Update FileInfo
        let updated_file_info = FileInfo {
            uuid,
            sha256: new_sha.clone(),
            path: new_persisted_path.to_string_lossy().to_string(),
            mime_type: new_mime_type,
        };

        // Update Redis: Remove old SHA entry and add new SHA entry
        redis_client.delete_file_info(&old_file_info.sha256).await?;
        redis_client.set_file_info(&new_sha, &updated_file_info).await?;
        redis_client.set_sha_uuid_mapping(&uuid, &new_sha).await?;

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
    pub async fn delete(uuid: Uuid, redis_client: &RedisClient) -> Result<(), FileError> {
        // Retrieve FileInfo to get SHA256 and path
        let file_info = Self::get(uuid, redis_client).await?;

        // Delete the file from the filesystem
        let file_path = Path::new(&file_info.path);
        if file_path.exists() {
            tokio::fs::remove_file(file_path).await.map_err(FileError::Io)?;
            info!("Deleted file at path: {}", file_info.path);
        } else {
            info!("File path does not exist, skipping deletion: {}", file_info.path);
        }

        // Delete the FileInfo from Redis
        redis_client.delete_file_info(&file_info.sha256).await?;

        // Delete the UUID to SHA mapping
        redis_client.delete_sha_uuid_mapping(&uuid).await?;

        // Remove the UUID directory if empty
        let uuid_dir = file_path.parent().ok_or(FileError::FileNotFound(uuid))?;
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
}

/// Sanitizes the file name to prevent security vulnerabilities like directory traversal.
/// Replaces any non-alphanumeric characters (excluding '.' and '_') with underscores.
fn sanitize_file_name(file_name: &str) -> String {
    file_name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' })
        .collect()
}
