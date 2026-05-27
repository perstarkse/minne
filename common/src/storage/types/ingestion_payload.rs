#![allow(
    clippy::result_large_err,
    clippy::needless_pass_by_value,
    clippy::implicit_clone,
    clippy::semicolon_if_nothing_returned
)]
use crate::{error::AppError, storage::types::file_info::FileInfo};
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum IngestionPayload {
    Url {
        url: String,
        context: String,
        category: String,
        user_id: String,
    },
    Text {
        text: String,
        context: String,
        category: String,
        user_id: String,
    },
    File {
        file_info: FileInfo,
        context: String,
        category: String,
        user_id: String,
    },
}

impl IngestionPayload {
    /// Creates ingestion payloads from the provided content, context, and files.
    ///
    /// # Arguments
    /// * `content` - Optional textual content to be ingressed
    /// * `context` - context for processing the ingress content
    /// * `category` - Category to classify the ingressed content
    /// * `files` - Vector of `FileInfo` objects containing information about uploaded files
    /// * `user_id` - Identifier of the user performing the ingress operation
    ///
    /// # Returns
    /// * `Result<Vec<IngestionPayload>, AppError>` - On success, returns a vector of ingress objects
    ///   (one per file/content type). On failure, returns an `AppError`.
    #[allow(clippy::similar_names)]
    pub fn create_ingestion_payload(
        content: Option<String>,
        context: String,
        category: String,
        files: &[FileInfo],
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
                        context: context.clone(),
                        category: category.clone(),
                        user_id: user_id.into(),
                    });
                }
                Err(_) => {
                    if input_content.len() > 2 {
                        info!("Treating input as plain text");
                        object_list.push(IngestionPayload::Text {
                            text: input_content.to_string(),
                            context: context.clone(),
                            category: category.clone(),
                            user_id: user_id.into(),
                        });
                    }
                }
            }
        }

        for file in files {
            object_list.push(IngestionPayload::File {
                file_info: file.clone(),
                context: context.clone(),
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
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use anyhow::{self, Context};
    use chrono::Utc;

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
                user_id: "user123".to_string(),
                file_name: "mock.txt".to_string(),
                mime_type: "text/plain".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }
    }

    #[test]
    fn test_create_ingestion_payload_with_url() -> anyhow::Result<()> {
        let url = "https://example.com";
        let context = "Process this URL";
        let category = "websites";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            Some(url.to_string()),
            context.to_string(),
            category.to_string(),
            &files,
            user_id,
        )
        .with_context(|| "create_ingestion_payload".to_string())?;

        assert_eq!(result.len(), 1);
        match result.first().context("expected one result")? {
            IngestionPayload::Url {
                url: payload_url,
                context: payload_context,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                // URL parser may normalize the URL by adding a trailing slash
                assert!(payload_url == &url.to_string() || payload_url == &format!("{url}/"));
                assert_eq!(payload_context, &context);
                assert_eq!(payload_category, &category);
                assert_eq!(payload_user_id, &user_id);
            }
            _ => anyhow::bail!("Expected Url variant"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_with_text() -> anyhow::Result<()> {
        let text = "This is some text content";
        let context = "Process this text";
        let category = "notes";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            context.to_string(),
            category.to_string(),
            &files,
            user_id,
        )
        .with_context(|| "create_ingestion_payload".to_string())?;

        assert_eq!(result.len(), 1);
        match result.first().context("expected one result")? {
            IngestionPayload::Text {
                text: payload_text,
                context: payload_context,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                assert_eq!(payload_text, text);
                assert_eq!(payload_context, context);
                assert_eq!(payload_category, category);
                assert_eq!(payload_user_id, user_id);
            }
            _ => anyhow::bail!("Expected Text variant"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_with_file() -> anyhow::Result<()> {
        let context = "Process this file";
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
            context.to_string(),
            category.to_string(),
            &files,
            user_id,
        )
        .with_context(|| "create_ingestion_payload".to_string())?;

        assert_eq!(result.len(), 1);
        match result.first().context("expected one result")? {
            IngestionPayload::File {
                file_info: payload_file_info,
                context: payload_context,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                assert_eq!(payload_file_info.id, file_info.id);
                assert_eq!(payload_context, context);
                assert_eq!(payload_category, category);
                assert_eq!(payload_user_id, user_id);
            }
            _ => anyhow::bail!("Expected File variant"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_with_url_and_file() -> anyhow::Result<()> {
        let url = "https://example.com";
        let context = "Process this data";
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
            context.to_string(),
            category.to_string(),
            &files,
            user_id,
        )
        .with_context(|| "create_ingestion_payload".to_string())?;

        assert_eq!(result.len(), 2);

        // Check first item is URL
        match result.first().context("expected first item")? {
            IngestionPayload::Url {
                url: payload_url, ..
            } => {
                // URL parser may normalize the URL by adding a trailing slash
                assert!(payload_url == &url.to_string() || payload_url == &format!("{url}/"));
            }
            _ => anyhow::bail!("Expected first item to be Url variant"),
        }

        // Check second item is File
        match result.get(1).context("expected second item")? {
            IngestionPayload::File {
                file_info: payload_file_info,
                ..
            } => {
                assert_eq!(payload_file_info.id, file_info.id);
            }
            _ => anyhow::bail!("Expected second item to be File variant"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_empty_input() -> anyhow::Result<()> {
        let context = "Process something";
        let category = "empty";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            None,
            context.to_string(),
            category.to_string(),
            &files,
            user_id,
        );

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(msg)) => {
                assert_eq!(msg, "No valid content or files provided");
            }
            _ => anyhow::bail!("Expected NotFound error"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_with_empty_text() -> anyhow::Result<()> {
        let text = ""; // Empty text
        let context = "Process this";
        let category = "notes";
        let user_id = "user123";
        let files = vec![];

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            context.to_string(),
            category.to_string(),
            &files,
            user_id,
        );

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(msg)) => {
                assert_eq!(msg, "No valid content or files provided");
            }
            _ => anyhow::bail!("Expected NotFound error"),
        }
        Ok(())
    }
}
