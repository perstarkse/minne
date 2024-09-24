use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use url::Url;
use super::files::FileInfo;

#[derive(Debug, Deserialize, Serialize)]
pub enum Content {
    Url(String),
    Text(String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IngressInput {
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,
    pub files: Option<Vec<String>>,
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
    /// Create a new `IngressContent` from `IngressInput`.
    pub async fn new(
        input: IngressInput
    ) -> Result<IngressContent, IngressContentError> {
        let content = if let Some(input_content) = input.content {
            // Check if the content is a URL
            if let Ok(url) = Url::parse(&input_content) {
                info!("Detected URL: {}", url);
                Some(Content::Url(url.to_string()))
            } else {
                info!("Treating input as plain text");
                Some(Content::Text(input_content))
            }
        } else {
            None
        };

        // Check if there is files
        // Use FileInfo.get(id) with all the ids in files vec
        // Return a vec<FileInfo> as file_info_list

        Ok(IngressContent {
            content,
            instructions: input.instructions,
            category: input.category,
            files: None,
        })
    }
}
