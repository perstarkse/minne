use async_openai::error::OpenAIError;
use thiserror::Error;
use tokio::task::JoinError;

use crate::storage::types::file_info::FileError;

/// Errors from embedding provider operations.
#[allow(clippy::module_name_repetitions)]
#[derive(Error, Debug)]
pub enum EmbeddingError {
    #[error("openai error: {0}")]
    OpenAI(Box<OpenAIError>),
    #[error("fastembed error: {0}")]
    FastEmbed(String),
    #[error("task join error: {0}")]
    Join(#[from] JoinError),
    #[error("fastembed model mutex poisoned: {0}")]
    MutexPoisoned(String),
    #[error("no embedding data received")]
    NoData,
    #[error("embedding configuration error: {0}")]
    Config(String),
    #[error("unknown fastembed model: {0}")]
    UnknownModel(String),
}

impl From<OpenAIError> for EmbeddingError {
    fn from(err: OpenAIError) -> Self {
        Self::OpenAI(Box::new(err))
    }
}

impl EmbeddingError {
    pub(crate) fn fastembed(err: impl std::fmt::Display) -> Self {
        Self::FastEmbed(err.to_string())
    }

    pub(crate) fn mutex_poisoned(err: impl std::fmt::Display) -> Self {
        Self::MutexPoisoned(err.to_string())
    }
}

// Core internal errors
#[allow(clippy::module_name_repetitions)]
#[derive(Error, Debug)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(Box<surrealdb::Error>),
    #[error("openai error: {0}")]
    OpenAI(Box<OpenAIError>),
    #[error("embedding error: {0}")]
    Embedding(#[from] EmbeddingError),
    #[error("file error: {0}")]
    File(#[from] FileError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("authorization error: {0}")]
    Auth(String),
    #[error("llm parsing error: {0}")]
    LLMParsing(String),
    #[error("task join error: {0}")]
    Join(#[from] JoinError),
    #[error("graph mapper error: {0}")]
    GraphMapper(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("reqwest error: {0}")]
    Reqwest(Box<reqwest::Error>),
    #[error("storage error: {0}")]
    Storage(Box<object_store::Error>),
    #[error("ingestion processing error: {0}")]
    Processing(String),
    #[error("dom smoothie error: {0}")]
    DomSmoothie(Box<dom_smoothie::ReadabilityError>),
    #[error("internal service error: {0}")]
    InternalError(String),
}

impl From<surrealdb::Error> for AppError {
    fn from(err: surrealdb::Error) -> Self {
        Self::Database(Box::new(err))
    }
}

impl From<OpenAIError> for AppError {
    fn from(err: OpenAIError) -> Self {
        Self::OpenAI(Box::new(err))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        Self::Reqwest(Box::new(err))
    }
}

impl From<object_store::Error> for AppError {
    fn from(err: object_store::Error) -> Self {
        Self::Storage(Box::new(err))
    }
}

impl From<dom_smoothie::ReadabilityError> for AppError {
    fn from(err: dom_smoothie::ReadabilityError) -> Self {
        Self::DomSmoothie(Box::new(err))
    }
}

impl AppError {
    /// Builds an [`AppError::InternalError`] from a displayable message.
    #[must_use]
    pub fn internal(msg: impl std::fmt::Display) -> Self {
        Self::InternalError(msg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn app_error_is_reasonably_sized() {
        assert!(
            std::mem::size_of::<AppError>() <= 64,
            "AppError is {} bytes",
            std::mem::size_of::<AppError>()
        );
    }
}
