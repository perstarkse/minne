use async_openai::error::OpenAIError;
use thiserror::Error;
use tokio::task::JoinError;

use crate::{storage::types::file_info::FileError, utils::mailer::EmailError};

// Core internal errors
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] surrealdb::Error),
    #[error("OpenAI error: {0}")]
    OpenAI(#[from] OpenAIError),
    #[error("File error: {0}")]
    File(#[from] FileError),
    #[error("Email error: {0}")]
    Email(#[from] EmailError),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Authorization error: {0}")]
    Auth(String),
    #[error("LLM parsing error: {0}")]
    LLMParsing(String),
    #[error("Task join error: {0}")]
    Join(#[from] JoinError),
    #[error("Graph mapper error: {0}")]
    GraphMapper(String),
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    #[error("Minijina error: {0}")]
    MiniJinja(#[from] minijinja::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Tiktoken error: {0}")]
    Tiktoken(#[from] anyhow::Error),
    #[error("Ingress Processing error: {0}")]
    Processing(String),
}
