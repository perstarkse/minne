use super::config::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestValidationError {
    PayloadTooLarge(String),
    BadRequest(String),
}

pub fn validate_ingest_input(
    config: &AppConfig,
    content: Option<&str>,
    context: &str,
    category: &str,
    file_count: usize,
) -> Result<(), IngestValidationError> {
    if file_count > config.ingest_max_files {
        return Err(IngestValidationError::BadRequest(format!(
            "Too many files. Maximum allowed is {}",
            config.ingest_max_files
        )));
    }

    if let Some(content) = content {
        if content.len() > config.ingest_max_content_bytes {
            return Err(IngestValidationError::PayloadTooLarge(format!(
                "Content is too large. Maximum allowed is {} bytes",
                config.ingest_max_content_bytes
            )));
        }
    }

    if context.len() > config.ingest_max_context_bytes {
        return Err(IngestValidationError::PayloadTooLarge(format!(
            "Context is too large. Maximum allowed is {} bytes",
            config.ingest_max_context_bytes
        )));
    }

    if category.len() > config.ingest_max_category_bytes {
        return Err(IngestValidationError::PayloadTooLarge(format!(
            "Category is too large. Maximum allowed is {} bytes",
            config.ingest_max_category_bytes
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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
}
