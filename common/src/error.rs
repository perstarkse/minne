use async_openai::error::OpenAIError;
use thiserror::Error;
use tokio::task::JoinError;

use crate::storage::types::file_info::FileError;

/// Errors from embedding provider operations.
#[allow(clippy::module_name_repetitions)]
#[derive(Error, Debug)]
pub enum EmbeddingError {
    #[error("openai error: {0}")]
    OpenAI(#[from] OpenAIError),
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
    Database(#[from] surrealdb::Error),
    #[error("openai error: {0}")]
    OpenAI(#[from] OpenAIError),
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
    Reqwest(#[from] reqwest::Error),
    #[error("storage error: {0}")]
    Storage(#[from] object_store::Error),
    #[error("ingestion processing error: {0}")]
    Processing(String),
    #[error("dom smoothie error: {0}")]
    DomSmoothie(#[from] dom_smoothie::ReadabilityError),
    #[error("internal service error: {0}")]
    InternalError(String),
}

impl AppError {
    /// Builds an [`AppError::InternalError`] from a displayable message.
    #[must_use]
    pub fn internal(msg: impl std::fmt::Display) -> Self {
        Self::InternalError(msg.to_string())
    }
}
