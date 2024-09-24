use axum_typed_multipart::{FieldData, TryFromMultipart};
use mime_guess::from_path;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::{BufReader, Read}, path::Path};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::info;
use url::Url;
use uuid::Uuid;

/// Error types for file and content handling.
#[derive(Error, Debug)]
pub enum FileError{
    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("MIME type detection failed for input: {0}")]
    MimeDetection(String),

    #[error("Unsupported MIME type: {0}")]
    UnsupportedMime(String),
}

#[derive(Debug, TryFromMultipart)]
pub struct FileUploadRequest {
    #[form_data(limit = "unlimited")]
    pub file: FieldData<NamedTempFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    pub uuid: Uuid,
    pub sha256: String,
    pub path: String,
    pub mime_type: String,
}

impl FileInfo {
    pub async fn new(file: NamedTempFile) -> Result<FileInfo, FileError> {
        // Calculate SHA based on file
        let sha = Self::get_sha(&file).await?;
        // Check if SHA exists in redis db
        // If so, return existing FileInfo
        // Generate UUID
        // Persist file with uuid as path
        // Guess the mime_type
        // Construct the object
        // file.persist("./data/{id}");        
        Ok(FileInfo { uuid: Uuid::new_v4(), sha256: sha, path:String::new(), mime_type:String::new()  })
    }
    
    pub async fn get(id: String) -> Result<FileInfo, FileError> {
        // Get the SHA based on file in uuid path
        // Check if SHA exists in redis
        // If so, return FileInfo
        // Else return error
        Ok(FileInfo { uuid: Uuid::new_v4(), sha256: String::new() , path:String::new(), mime_type:String::new()  })
    }
    
    pub async fn update(id: String, file: NamedTempFile) -> Result<FileInfo, FileError> {
        // Calculate SHA based on file
        // Check if SHA exists in redis
        // If so, return existing FileInfo
        // Use the submitted UUID
        // Replace the old file with uuid as path
        // Guess the mime_type
        // Construct the object
        Ok(FileInfo { uuid: Uuid::new_v4(), sha256: String::new() , path:String::new(), mime_type:String::new()  })
    }

    pub async fn delete(id: String) -> Result<(), FileError> {
        // Get the SHA based on file in uuid path
        // Remove the entry from redis db
        Ok(())
    }

    async fn get_sha(file: &NamedTempFile) -> Result<String, FileError> {
        let input = file.as_file();
        let mut reader = BufReader::new(input);
        let digest = {
            let mut hasher = Sha256::new();
            let mut buffer = [0; 1024];
            loop {
                let count = reader.read(&mut buffer)?;
                if count == 0 { break }
                hasher.update(&buffer[..count]);
            }
            hasher.finalize()
        };
        
        Ok(format!("{:X}", digest))
    }
}
    // let input = File::open(path)?;
    // let mut reader = BufReader::new(input);

    // let digest = {
    //     let mut hasher = Sha256::new();
    //     let mut buffer = [0; 1024];
    //     loop {
    //         let count = reader.read(&mut buffer)?;
    //         if count == 0 { break }
    //         hasher.update(&buffer[..count]);
    //     }
    //     hasher.finalize()
    // };
    // Ok(HEXLOWER.encode(digest.as_ref()))

