use async_openai::error::OpenAIError;
use thiserror::Error;
use tokio::task::JoinError;

/// Error types for processing `TextContent`.
#[derive(Error, Debug)]
pub enum ProcessingError {
    #[error("SurrealDb error: {0}")]
    SurrealDbError(#[from] surrealdb::Error),

    #[error("LLM processing error: {0}")]
    OpenAIerror(#[from] OpenAIError),

    #[error("Embedding processing error: {0}")]
    EmbeddingError(String),

    #[error("Graph processing error: {0}")]
    GraphProcessingError(String),

    #[error("LLM parsing error: {0}")]
    LLMParsingError(String),

    #[error("Task join error: {0}")]
    JoinError(#[from] JoinError),
}
