use async_openai::error::OpenAIError;
use thiserror::Error;
use tokio::task::JoinError;

use crate::{ingress::types::ingress_input::IngressContentError, rabbitmq::RabbitMQError};

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

#[derive(Error, Debug)]
pub enum IngressConsumerError {
    #[error("RabbitMQ error: {0}")]
    RabbitMQ(#[from] RabbitMQError),

    #[error("Processing error: {0}")]
    Processing(#[from] ProcessingError),

    #[error("Ingress content error: {0}")]
    IngressContent(#[from] IngressContentError),
}
