use axum_typed_multipart::FieldData;
use mime_guess::from_path;
use sha2::{Digest, Sha256};
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

use crate::{
    storage::db::{store_item, SurrealDbClient},
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
}

stored_object!(FileInfo, "file", {
    sha256: String,
    path: String,
    file_name: String,
    mime_type: String
});

impl FileInfo {
    pub async fn new(
        field_data: FieldData<NamedTempFile>,
        db_client: &SurrealDbClient,
        user_id: &str,
    ) -> Result<Self, FileError> {
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
            path: Self::persist_file(&uuid, file, &sanitized_file_name, user_id)
                .await?
                .to_string_lossy()
                .into(),
            mime_type: Self::guess_mime_type(Path::new(&sanitized_file_name)),
        };

        // Store in database
        store_item(&db_client.client, file_info.clone()).await?;

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
        file_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// Persists the file to the filesystem under `./data/{user_id}/{uuid}/{file_name}`.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file.
    /// * `file` - The temporary file to persist.
    /// * `file_name` - The sanitized file name.
    /// * `user-id` - User id
    ///
    /// # Returns
    /// * `Result<PathBuf, FileError>` - The persisted file path or an error.
    async fn persist_file(
        uuid: &Uuid,
        file: NamedTempFile,
        file_name: &str,
        user_id: &str,
    ) -> Result<PathBuf, FileError> {
        let base_dir = Path::new("./data");
        let user_dir = base_dir.join(user_id); // Create the user directory
        let uuid_dir = user_dir.join(uuid.to_string()); // Create the UUID directory under the user directory

        // Create the user and UUID directories if they don't exist
        tokio::fs::create_dir_all(&uuid_dir)
            .await
            .map_err(FileError::Io)?;

        // Define the final file path
        let final_path = uuid_dir.join(file_name);
        info!("Final path: {:?}", final_path);

        // Persist the temporary file to the final path
        file.persist(&final_path)?;
        info!("Persisted file to {:?}", final_path);

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
}
