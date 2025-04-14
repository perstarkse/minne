use crate::{error::AppError, storage::types::file_info::FileInfo};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    // Create a mock FileInfo for testing
    #[derive(Debug, Clone, PartialEq)]
    struct MockFileInfo {
        id: String,
    }

    impl From<MockFileInfo> for FileInfo {
        fn from(mock: MockFileInfo) -> Self {
            // This is just a test implementation, the actual fields don't matter
            // as we're just testing the IngestionPayload functionality
            FileInfo {
                id: mock.id,
                sha256: "mock-sha256".to_string(),
                path: "/mock/path".to_string(),
                file_name: "mock.txt".to_string(),
                mime_type: "text/plain".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }
    }

    #[test]
    fn test_create_ingestion_payload_with_url() {
        let url = "https://example.com";
        let instructions = "Process this URL";
        let category = "websites";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            Some(url.to_string()),
            instructions.to_string(),
            category.to_string(),
            files,
            user_id,
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        match &result[0] {
            IngestionPayload::Url {
                url: payload_url,
                instructions: payload_instructions,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                // URL parser may normalize the URL by adding a trailing slash
                assert!(payload_url == &url.to_string() || payload_url == &format!("{}/", url));
                assert_eq!(payload_instructions, &instructions);
                assert_eq!(payload_category, &category);
                assert_eq!(payload_user_id, &user_id);
            }
            _ => panic!("Expected Url variant"),
        }
    }

    #[test]
    fn test_create_ingestion_payload_with_text() {
        let text = "This is some text content";
        let instructions = "Process this text";
        let category = "notes";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            instructions.to_string(),
            category.to_string(),
            files,
            user_id,
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        match &result[0] {
            IngestionPayload::Text {
                text: payload_text,
                instructions: payload_instructions,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                assert_eq!(payload_text, text);
                assert_eq!(payload_instructions, instructions);
                assert_eq!(payload_category, category);
                assert_eq!(payload_user_id, user_id);
            }
            _ => panic!("Expected Text variant"),
        }
    }

    #[test]
    fn test_create_ingestion_payload_with_file() {
        let instructions = "Process this file";
        let category = "documents";
        let user_id = "user123";

        // Create a mock FileInfo
        let mock_file = MockFileInfo {
            id: "file123".to_string(),
        };

        let file_info: FileInfo = mock_file.into();
        let files = vec![file_info.clone()];

        let result = IngestionPayload::create_ingestion_payload(
            None,
            instructions.to_string(),
            category.to_string(),
            files,
            user_id,
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        match &result[0] {
            IngestionPayload::File {
                file_info: payload_file_info,
                instructions: payload_instructions,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                assert_eq!(payload_file_info.id, file_info.id);
                assert_eq!(payload_instructions, instructions);
                assert_eq!(payload_category, category);
                assert_eq!(payload_user_id, user_id);
            }
            _ => panic!("Expected File variant"),
        }
    }

    #[test]
    fn test_create_ingestion_payload_with_url_and_file() {
        let url = "https://example.com";
        let instructions = "Process this data";
        let category = "mixed";
        let user_id = "user123";

        // Create a mock FileInfo
        let mock_file = MockFileInfo {
            id: "file123".to_string(),
        };

        let file_info: FileInfo = mock_file.into();
        let files = vec![file_info.clone()];

        let result = IngestionPayload::create_ingestion_payload(
            Some(url.to_string()),
            instructions.to_string(),
            category.to_string(),
            files,
            user_id,
        )
        .unwrap();

        assert_eq!(result.len(), 2);

        // Check first item is URL
        match &result[0] {
            IngestionPayload::Url {
                url: payload_url, ..
            } => {
                // URL parser may normalize the URL by adding a trailing slash
                assert!(payload_url == &url.to_string() || payload_url == &format!("{}/", url));
            }
            _ => panic!("Expected first item to be Url variant"),
        }

        // Check second item is File
        match &result[1] {
            IngestionPayload::File {
                file_info: payload_file_info,
                ..
            } => {
                assert_eq!(payload_file_info.id, file_info.id);
            }
            _ => panic!("Expected second item to be File variant"),
        }
    }

    #[test]
    fn test_create_ingestion_payload_empty_input() {
        let instructions = "Process something";
        let category = "empty";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            None,
            instructions.to_string(),
            category.to_string(),
            files,
            user_id,
        );

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(msg)) => {
                assert_eq!(msg, "No valid content or files provided");
            }
            _ => panic!("Expected NotFound error"),
        }
    }

    #[test]
    fn test_create_ingestion_payload_with_empty_text() {
        let text = ""; // Empty text
        let instructions = "Process this";
        let category = "notes";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            instructions.to_string(),
            category.to_string(),
            files,
            user_id,
        );

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(msg)) => {
                assert_eq!(msg, "No valid content or files provided");
            }
            _ => panic!("Expected NotFound error"),
        }
    }
}
