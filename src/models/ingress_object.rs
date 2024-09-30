use crate::models::file_info::FileInfo;
use serde::{Deserialize, Serialize};

use super::{ingress_content::IngressContentError, text_content::TextContent};

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
    pub async fn to_text_content(&self) -> Result<TextContent, IngressContentError> {
        match self {
            IngressObject::Url { url, instructions, category } => {
                let text = Self::fetch_text_from_url(url).await?;
                Ok(TextContent {
                    text,
                    instructions: instructions.clone(),
                    category: category.clone(),
                    file_info: None,
                })
            },
            IngressObject::Text { text, instructions, category } => {
                Ok(TextContent {
                    text: text.clone(),
                    instructions: instructions.clone(),
                    category: category.clone(),
                    file_info: None,
                })
            },
            IngressObject::File { file_info, instructions, category } => {
                let text = Self::extract_text_from_file(file_info).await?;
                Ok(TextContent {
                    text,
                    instructions: instructions.clone(),
                    category: category.clone(),
                    file_info: Some(file_info.clone()),
                })
            },
        }
    }

    /// Fetches and extracts text from a URL.
    async fn fetch_text_from_url(url: &str) -> Result<String, IngressContentError> {
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
            "application/pdf" => {
                // TODO: Implement PDF text extraction using a crate like `pdf-extract` or `lopdf`
                Err(IngressContentError::UnsupportedMime(file_info.mime_type.clone()))
            }
            "image/png" | "image/jpeg" => {
                // TODO: Implement OCR on image using a crate like `tesseract`
                Err(IngressContentError::UnsupportedMime(file_info.mime_type.clone()))
            }
            // Handle other MIME types as needed
            _ => Err(IngressContentError::UnsupportedMime(file_info.mime_type.clone())),
        }
    }
}

