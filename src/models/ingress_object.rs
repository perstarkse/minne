use super::ingress_content::IngressContentError;
use crate::models::file_info::FileInfo;
use crate::storage::types::text_content::TextContent;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Knowledge object type, containing the content or reference to it, as well as metadata
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum IngressObject {
    Url {
        url: String,
        instructions: String,
        category: String,
    },
    Text {
        text: String,
        instructions: String,
        category: String,
    },
    File {
        file_info: FileInfo,
        instructions: String,
        category: String,
    },
}

impl IngressObject {
    /// Creates a new `TextContent` instance from a `IngressObject`.
    ///
    /// # Arguments
    /// `&self` - A reference to the `IngressObject`.
    ///
    /// # Returns
    /// `TextContent` - An object containing a text representation of the object, could be a scraped URL, parsed PDF, etc.
    pub async fn to_text_content(&self) -> Result<TextContent, IngressContentError> {
        match self {
            IngressObject::Url {
                url,
                instructions,
                category,
            } => {
                let text = Self::fetch_text_from_url(url).await?;
                Ok(TextContent::new(
                    text,
                    instructions.into(),
                    category.into(),
                    None,
                ))
            }
            IngressObject::Text {
                text,
                instructions,
                category,
            } => Ok(TextContent::new(
                text.into(),
                instructions.into(),
                category.into(),
                None,
            )),
            IngressObject::File {
                file_info,
                instructions,
                category,
            } => {
                let text = Self::extract_text_from_file(file_info).await?;
                Ok(TextContent::new(
                    text,
                    instructions.into(),
                    category.into(),
                    Some(file_info.to_owned()),
                ))
            }
        }
    }

    /// Fetches and extracts text from a URL.
    async fn fetch_text_from_url(_url: &str) -> Result<String, IngressContentError> {
        unimplemented!()
    }

    /// Extracts text from a file based on its MIME type.
    async fn extract_text_from_file(file_info: &FileInfo) -> Result<String, IngressContentError> {
        match file_info.mime_type.as_str() {
            "text/plain" => {
                // Read the file and return its content
                let content = tokio::fs::read_to_string(&file_info.path).await?;
                Ok(content)
            }
            "text/markdown" => {
                // Read the file and return its content
                let content = tokio::fs::read_to_string(&file_info.path).await?;
                Ok(content)
            }
            "application/pdf" => {
                // TODO: Implement PDF text extraction using a crate like `pdf-extract` or `lopdf`
                Err(IngressContentError::UnsupportedMime(
                    file_info.mime_type.clone(),
                ))
            }
            "image/png" | "image/jpeg" => {
                // TODO: Implement OCR on image using a crate like `tesseract`
                Err(IngressContentError::UnsupportedMime(
                    file_info.mime_type.clone(),
                ))
            }
            "application/octet-stream" => {
                let content = tokio::fs::read_to_string(&file_info.path).await?;
                Ok(content)
            }
            "text/x-rust" => {
                let content = tokio::fs::read_to_string(&file_info.path).await?;
                Ok(content)
            }
            // Handle other MIME types as needed
            _ => Err(IngressContentError::UnsupportedMime(
                file_info.mime_type.clone(),
            )),
        }
    }
}
