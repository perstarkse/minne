use axum::extract::Multipart;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;
use tracing::info;
use url::Url;
use uuid::Uuid;
use sha2::{Digest, Sha256};
use std::path::Path;
use mime_guess::from_path;
use axum_typed_multipart::{FieldData, TryFromMultipart };
use tempfile::NamedTempFile;

#[derive(Debug, TryFromMultipart)]
pub struct IngressMultipart {
    /// JSON content field
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,

    /// Optional file
    #[form_data(limit = "unlimited")]
    pub file: Option<FieldData<NamedTempFile>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    pub uuid: Uuid,
    pub sha256: String,
    pub path: String,
    pub mime_type: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Content {
    Url(String),
    Text(String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IngressContent {
    pub content: Option<Content>,
    pub instructions: String,
    pub category: String,
    pub files: Option<Vec<FileInfo>>,
}

/// Error types for file and content handling.
#[derive(Error, Debug)]
pub enum IngressContentError {
    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("MIME type detection failed for input: {0}")]
    MimeDetection(String),

    #[error("Unsupported MIME type: {0}")]
    UnsupportedMime(String),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
}

impl IngressContent {
    /// Create a new `IngressContent` from `IngressMultipart`.
    pub async fn new(
        content: Option<String>, instructions: String, category: String,
        file: Option<FileInfo>
    ) -> Result<IngressContent, IngressContentError> {
        let content = if let Some(content_str) = content {
            // Check if the content is a URL
            if let Ok(url) = Url::parse(&content_str) {
                info!("Detected URL: {}", url);
                Some(Content::Url(url.to_string()))
            } else {
                info!("Treating input as plain text");
                Some(Content::Text(content_str))
            }
        } else {
            None
        };

        Ok(IngressContent {
            content,
            instructions,
            category,
            files: file.map(|f| vec![f]), // Single file wrapped in a Vec
        })
    }
}
