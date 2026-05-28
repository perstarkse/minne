#![allow(clippy::result_large_err)]
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

/// Shared ingest metadata moved or cloned into each payload variant.
struct IngestFields {
    context: String,
    category: String,
    user_id: String,
}

/// Result of parsing optional ingest content before file payloads are built.
#[derive(Debug)]
enum ParsedContent {
    /// No URL or text payload should be appended.
    Skip,
    Url(String),
    Text(String),
}

impl ParsedContent {
    #[must_use]
    fn follows(&self) -> bool {
        !matches!(self, Self::Skip)
    }
}

impl IngestionPayload {
    /// Creates ingestion payloads from the provided content, context, and files.
    ///
    /// Files are emitted first. When both files and content are present, shared
    /// metadata is cloned per file; otherwise the last file-only payload moves
    /// `context`, `category`, and `user_id` without cloning.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::NotFound`] when no valid files or content are provided.
    #[allow(clippy::similar_names)]
    pub fn create_ingestion_payload(
        content: Option<String>,
        context: String,
        category: String,
        files: Vec<FileInfo>,
        user_id: String,
    ) -> Result<Vec<IngestionPayload>, AppError> {
        let parsed = Self::parse_content(content);
        let content_follows = parsed.follows();
        let file_count = files.len();
        #[allow(clippy::arithmetic_side_effects)]
        let capacity = file_count + usize::from(content_follows);
        let mut object_list = Vec::with_capacity(capacity);
        let mut fields = Some(IngestFields {
            context,
            category,
            user_id,
        });

        for (index, file) in files.into_iter().enumerate() {
            let is_last_file = index + 1 == file_count;
            if content_follows || !is_last_file {
                let Some(shared) = fields.as_ref() else {
                    return Err(AppError::internal("shared ingest fields consumed early"));
                };
                object_list.push(Self::File {
                    file_info: file,
                    context: shared.context.clone(),
                    category: shared.category.clone(),
                    user_id: shared.user_id.clone(),
                });
            } else {
                let Some(shared) = fields.take() else {
                    return Err(AppError::internal("shared ingest fields missing for file"));
                };
                object_list.push(Self::File {
                    file_info: file,
                    context: shared.context,
                    category: shared.category,
                    user_id: shared.user_id,
                });
            }
        }

        if let ParsedContent::Url(url) = parsed {
            info!("Detected URL: {url}");
            let Some(shared) = fields.take() else {
                return Err(AppError::internal("shared ingest fields missing for url"));
            };
            object_list.push(Self::Url {
                url,
                context: shared.context,
                category: shared.category,
                user_id: shared.user_id,
            });
        } else if let ParsedContent::Text(text) = parsed {
            info!("Treating input as plain text");
            let Some(shared) = fields.take() else {
                return Err(AppError::internal("shared ingest fields missing for text"));
            };
            object_list.push(Self::Text {
                text,
                context: shared.context,
                category: shared.category,
                user_id: shared.user_id,
            });
        }

        if object_list.is_empty() {
            return Err(AppError::NotFound(
                "no valid content or files provided".into(),
            ));
        }

        Ok(object_list)
    }

    fn parse_content(content: Option<String>) -> ParsedContent {
        let Some(input_content) = content else {
            return ParsedContent::Skip;
        };

        if input_content.len() <= 2 {
            return ParsedContent::Skip;
        }

        match Url::parse(&input_content) {
            Ok(url) => ParsedContent::Url(url.to_string()),
            Err(_) => ParsedContent::Text(input_content),
        }
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

        let result = IngestionPayload::create_ingestion_payload(
            Some(url.to_string()),
            context.to_string(),
            category.to_string(),
            vec![],
            user_id.to_string(),
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

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            context.to_string(),
            category.to_string(),
            vec![],
            user_id.to_string(),
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
        let file_id = file_info.id.clone();

        let result = IngestionPayload::create_ingestion_payload(
            None,
            context.to_string(),
            category.to_string(),
            vec![file_info],
            user_id.to_string(),
        )?;

        assert_eq!(result.len(), 1);
        match result.first().context("expected one result")? {
            IngestionPayload::File {
                file_info: payload_file_info,
                context: payload_context,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                assert_eq!(payload_file_info.id, file_id);
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
        let file_id = file_info.id.clone();

        let result = IngestionPayload::create_ingestion_payload(
            Some(url.to_string()),
            context.to_string(),
            category.to_string(),
            vec![file_info],
            user_id.to_string(),
        )?;

        assert_eq!(result.len(), 2);

        // Check first item is File (files processed first to minimize clones)
        match result.first().context("expected first item")? {
            IngestionPayload::File {
                file_info: payload_file_info,
                ..
            } => {
                assert_eq!(payload_file_info.id, file_id);
            }
            _ => anyhow::bail!("Expected first item to be File variant"),
        }

        // Check second item is URL
        match result.get(1).context("expected second item")? {
            IngestionPayload::Url {
                url: payload_url, ..
            } => {
                // URL parser may normalize the URL by adding a trailing slash
                assert!(payload_url == &url.to_string() || payload_url == &format!("{url}/"));
            }
            _ => anyhow::bail!("Expected second item to be Url variant"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_empty_input() -> anyhow::Result<()> {
        let context = "Process something";
        let category = "empty";
        let user_id = "user123";

        let result = IngestionPayload::create_ingestion_payload(
            None,
            context.to_string(),
            category.to_string(),
            vec![],
            user_id.to_string(),
        );

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(msg)) => {
                assert_eq!(msg, "no valid content or files provided");
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

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            context.to_string(),
            category.to_string(),
            vec![],
            user_id.to_string(),
        );

        assert!(result.is_err());
        match result {
            Err(AppError::NotFound(msg)) => {
                assert_eq!(msg, "no valid content or files provided");
            }
            _ => anyhow::bail!("Expected NotFound error"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_with_file_and_text() -> anyhow::Result<()> {
        let text = "plain notes";
        let context = "ctx";
        let category = "cat";
        let user_id = "user123";
        let file_info: FileInfo = MockFileInfo {
            id: "file1".to_string(),
        }
        .into();

        let result = IngestionPayload::create_ingestion_payload(
            Some(text.to_string()),
            context.to_string(),
            category.to_string(),
            vec![file_info],
            user_id.to_string(),
        )?;

        assert_eq!(result.len(), 2);
        match (&result[0], &result[1]) {
            (
                IngestionPayload::File {
                    file_info: payload_file,
                    context: file_context,
                    ..
                },
                IngestionPayload::Text {
                    text: payload_text,
                    context: text_context,
                    category: text_category,
                    user_id: text_user_id,
                },
            ) => {
                assert_eq!(payload_file.id, "file1");
                assert_eq!(file_context, context);
                assert_eq!(payload_text, text);
                assert_eq!(text_context, context);
                assert_eq!(text_category, category);
                assert_eq!(text_user_id, user_id);
            }
            _ => anyhow::bail!("expected File then Text"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_short_content_with_file_only_yields_file() -> anyhow::Result<()> {
        let context = "ctx";
        let category = "cat";
        let user_id = "user123";
        let file_info: FileInfo = MockFileInfo {
            id: "file1".to_string(),
        }
        .into();

        let result = IngestionPayload::create_ingestion_payload(
            Some("ab".to_string()),
            context.to_string(),
            category.to_string(),
            vec![file_info],
            user_id.to_string(),
        )?;

        assert_eq!(result.len(), 1);
        match result.first().context("expected one file payload")? {
            IngestionPayload::File {
                file_info,
                context: payload_context,
                category: payload_category,
                user_id: payload_user_id,
            } => {
                assert_eq!(file_info.id, "file1");
                assert_eq!(payload_context, context);
                assert_eq!(payload_category, category);
                assert_eq!(payload_user_id, user_id);
            }
            _ => anyhow::bail!("expected File variant only"),
        }
        Ok(())
    }

    #[test]
    fn test_create_ingestion_payload_two_files_without_content() -> anyhow::Result<()> {
        let context = "ctx";
        let category = "cat";
        let user_id = "user123";

        let files = vec![
            MockFileInfo {
                id: "file1".to_string(),
            }
            .into(),
            MockFileInfo {
                id: "file2".to_string(),
            }
            .into(),
        ];

        let result = IngestionPayload::create_ingestion_payload(
            None,
            context.to_string(),
            category.to_string(),
            files,
            user_id.to_string(),
        )?;

        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], IngestionPayload::File { .. }));
        assert!(matches!(result[1], IngestionPayload::File { .. }));
        Ok(())
    }
}
