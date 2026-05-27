use async_openai::error::OpenAIError;
use thiserror::Error;
use tokio::task::JoinError;

use crate::storage::types::file_info::FileError;

// Core internal errors
#[allow(clippy::module_name_repetitions)]
#[derive(Error, Debug)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] surrealdb::Error),
    #[error("openai error: {0}")]
    OpenAI(#[from] OpenAIError),
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
