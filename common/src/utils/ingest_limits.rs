use thiserror::Error;

use super::config::AppConfig;

/// Errors raised when validating ingestion payloads against configured limits.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum IngestValidationError {
    /// The payload exceeds a configured size limit (content, context, or category).
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),
    /// The request violates a non-size constraint (e.g., too many files).
    #[error("bad request: {0}")]
    BadRequest(String),
}

/// Validates ingestion input against configured limits.
///
/// Checks file count, content size, context size, and category length.
///
/// # Errors
///
/// Returns `IngestValidationError::BadRequest` if the file count exceeds the maximum.
/// Returns `IngestValidationError::PayloadTooLarge` if content, context, or
/// category exceed their configured byte limits.
pub fn validate_ingest_input(
    config: &AppConfig,
    content: Option<&str>,
    ctx: &str,
    category: &str,
    file_count: usize,
) -> Result<(), IngestValidationError> {
    let content_bytes = content.map_or(0, str::len);
    let text_field_bytes = content_bytes
        .saturating_add(ctx.len())
        .saturating_add(category.len());
    if text_field_bytes > config.ingest_max_body_bytes {
        return Err(IngestValidationError::PayloadTooLarge(format!(
            "request text fields exceed maximum allowed body size of {} bytes",
            config.ingest_max_body_bytes
        )));
    }

    if file_count > config.ingest_max_files {
        return Err(IngestValidationError::BadRequest(format!(
            "too many files: maximum allowed is {}",
            config.ingest_max_files
        )));
    }

    if let Some(content) = content {
        if content.len() > config.ingest_max_content_bytes {
            return Err(IngestValidationError::PayloadTooLarge(format!(
                "content is too large: maximum allowed is {} bytes",
                config.ingest_max_content_bytes
            )));
        }
    }

    if ctx.len() > config.ingest_max_context_bytes {
        return Err(IngestValidationError::PayloadTooLarge(format!(
            "context is too large: maximum allowed is {} bytes",
            config.ingest_max_context_bytes
        )));
    }

    if category.len() > config.ingest_max_category_bytes {
        return Err(IngestValidationError::PayloadTooLarge(format!(
            "category is too large: maximum allowed is {} bytes",
            config.ingest_max_category_bytes
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use super::*;

    #[test]
    fn validate_ingest_input_rejects_too_many_files() {
        let config = AppConfig {
            ingest_max_files: 1,
            ..Default::default()
        };
        let result = validate_ingest_input(&config, Some("ok"), "ctx", "cat", 2);

        assert!(matches!(result, Err(IngestValidationError::BadRequest(_))));
    }

    #[test]
    fn validate_ingest_input_rejects_oversized_content() {
        let config = AppConfig {
            ingest_max_content_bytes: 4,
            ..Default::default()
        };
        let result = validate_ingest_input(&config, Some("12345"), "ctx", "cat", 0);

        assert!(matches!(
            result,
            Err(IngestValidationError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn validate_ingest_input_rejects_oversized_context() {
        let config = AppConfig {
            ingest_max_context_bytes: 2,
            ..Default::default()
        };
        let result = validate_ingest_input(&config, None, "long", "cat", 0);

        assert!(matches!(
            result,
            Err(IngestValidationError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn validate_ingest_input_rejects_oversized_category() {
        let config = AppConfig {
            ingest_max_category_bytes: 2,
            ..Default::default()
        };
        let result = validate_ingest_input(&config, None, "ok", "long", 0);

        assert!(matches!(
            result,
            Err(IngestValidationError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn validate_ingest_input_accepts_valid_payload() {
        let config = AppConfig::default();
        let result = validate_ingest_input(&config, Some("ok"), "ctx", "cat", 1);

        assert!(result.is_ok());
    }

    #[test]
    fn validate_ingest_input_rejects_oversized_text_fields() {
        let config = AppConfig {
            ingest_max_body_bytes: 10,
            ..Default::default()
        };
        let result = validate_ingest_input(&config, Some("123456"), "ctx", "cat", 0);

        assert!(matches!(
            result,
            Err(IngestValidationError::PayloadTooLarge(_))
        ));
    }
}
