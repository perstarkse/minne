use crate::{error::AppError, storage::types::file_info::FileInfo};
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum IngestionPayload {
    Url {
        url: String,
        instructions: String,
        category: String,
        user_id: String,
    },
    Text {
        text: String,
        instructions: String,
        category: String,
        user_id: String,
    },
    File {
        file_info: FileInfo,
        instructions: String,
        category: String,
        user_id: String,
    },
}

impl IngestionPayload {
    /// Creates ingestion payloads from the provided content, instructions, and files.
    ///
    /// # Arguments
    /// * `content` - Optional textual content to be ingressed
    /// * `instructions` - Instructions for processing the ingress content
    /// * `category` - Category to classify the ingressed content
    /// * `files` - Vector of `FileInfo` objects containing information about uploaded files
    /// * `user_id` - Identifier of the user performing the ingress operation
    ///
    /// # Returns
    /// * `Result<Vec<IngestionPayload>, AppError>` - On success, returns a vector of ingress objects
    ///   (one per file/content type). On failure, returns an `AppError`.
    pub fn create_ingestion_payload(
        content: Option<String>,
        instructions: String,
        category: String,
        files: Vec<FileInfo>,
        user_id: &str,
    ) -> Result<Vec<IngestionPayload>, AppError> {
        // Initialize list
        let mut object_list = Vec::new();

        // Create a IngestionPayload from content if it exists, checking for URL or text
        if let Some(input_content) = content {
            match Url::parse(&input_content) {
                Ok(url) => {
                    info!("Detected URL: {}", url);
                    object_list.push(IngestionPayload::Url {
                        url: url.to_string(),
                        instructions: instructions.clone(),
                        category: category.clone(),
                        user_id: user_id.into(),
                    });
                }
                Err(_) => {
                    if input_content.len() > 2 {
                        info!("Treating input as plain text");
                        object_list.push(IngestionPayload::Text {
                            text: input_content.to_string(),
                            instructions: instructions.clone(),
                            category: category.clone(),
                            user_id: user_id.into(),
                        });
                    }
                }
            }
        }

        for file in files {
            object_list.push(IngestionPayload::File {
                file_info: file,
                instructions: instructions.clone(),
                category: category.clone(),
                user_id: user_id.into(),
            })
        }

        // If no objects are constructed, we return Err
        if object_list.is_empty() {
            return Err(AppError::NotFound(
                "No valid content or files provided".into(),
            ));
        }

        Ok(object_list)
    }
}
