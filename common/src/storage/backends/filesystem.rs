use super::{FileMetadata, StorageBackend, StorageError, StoredFile};
use async_trait::async_trait;
use bytes::Bytes;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

/// Filesystem-based storage backend
#[derive(Debug, Clone)]
pub struct FilesystemBackend {
    base_dir: PathBuf,
}

impl FilesystemBackend {
    pub fn new(data_dir: String) -> Result<Self, StorageError> {
        let base_dir = if data_dir.starts_with('/') {
            PathBuf::from(data_dir)
        } else {
            std::env::current_dir()
                .map_err(StorageError::Io)?
                .join(data_dir)
        };

        Ok(Self { base_dir })
    }

    /// Generate the storage path for a file: {base_dir}/{user_id}/{file_id}/{filename}
    fn get_file_path(&self, user_id: &str, file_id: &Uuid, filename: &str) -> PathBuf {
        self.base_dir
            .join(user_id)
            .join(file_id.to_string())
            .join(filename)
    }

    /// Parse a storage path back into components
    fn parse_storage_path(&self, path: &str) -> Result<PathBuf, StorageError> {
        let full_path = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.base_dir.join(path)
        };
        Ok(full_path)
    }
}

#[async_trait]
impl StorageBackend for FilesystemBackend {
    async fn store_file(
        &self,
        user_id: &str,
        file_id: &Uuid,
        filename: &str,
        content: Bytes,
        _content_type: Option<&str>,
    ) -> Result<String, StorageError> {
        let file_path = self.get_file_path(user_id, file_id, filename);

        // Create parent directories
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Write the file
        fs::write(&file_path, content).await?;

        // Return the relative path from base_dir for storage in database
        let relative_path = file_path.strip_prefix(&self.base_dir)
            .map_err(|_| StorageError::Config("Invalid file path".to_string()))?;
        
        Ok(relative_path.to_string_lossy().to_string())
    }

    async fn get_file(&self, path: &str) -> Result<StoredFile, StorageError> {
        let file_path = self.parse_storage_path(path)?;
        
        if !file_path.exists() {
            return Err(StorageError::NotFound(path.to_string()));
        }

        let content = fs::read(&file_path).await?;
        let metadata = self.get_metadata(path).await?;

        Ok(StoredFile {
            content: Bytes::from(content),
            metadata,
        })
    }

    async fn delete_file(&self, path: &str) -> Result<(), StorageError> {
        let file_path = self.parse_storage_path(path)?;
        
        if file_path.exists() {
            fs::remove_file(&file_path).await?;
            
            // Try to remove empty parent directories
            if let Some(parent) = file_path.parent() {
                let _ = fs::remove_dir(parent).await; // Ignore errors if directory not empty
                if let Some(grandparent) = parent.parent() {
                    let _ = fs::remove_dir(grandparent).await; // Ignore errors if directory not empty
                }
            }
        }
        
        Ok(())
    }

    async fn file_exists(&self, path: &str) -> Result<bool, StorageError> {
        let file_path = self.parse_storage_path(path)?;
        Ok(file_path.exists())
    }

    async fn get_metadata(&self, path: &str) -> Result<FileMetadata, StorageError> {
        let file_path = self.parse_storage_path(path)?;
        
        if !file_path.exists() {
            return Err(StorageError::NotFound(path.to_string()));
        }

        let metadata = fs::metadata(&file_path).await?;
        
        let last_modified = metadata.modified()
            .ok()
            .and_then(|time| chrono::DateTime::from_timestamp(
                time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64, 
                0
            ));

        // Try to guess content type from file extension
        let content_type = file_path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| mime_guess::from_ext(ext).first())
            .map(|mime| mime.to_string());

        Ok(FileMetadata {
            size: metadata.len(),
            content_type,
            last_modified,
        })
    }

    async fn list_files(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let search_path = self.base_dir.join(prefix);
        let mut files = Vec::new();

        if search_path.is_dir() {
            let mut entries = fs::read_dir(&search_path).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(relative_path) = path.strip_prefix(&self.base_dir) {
                        files.push(relative_path.to_string_lossy().to_string());
                    }
                }
            }
        }

        Ok(files)
    }
}