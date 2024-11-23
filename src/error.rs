use async_openai::error::OpenAIError;
use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use thiserror::Error;
use tokio::task::JoinError;

use crate::{ingress::types::ingress_input::IngressContentError, rabbitmq::RabbitMQError};

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

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("Processing error: {0}")]
    ProcessingError(#[from] ProcessingError),
    #[error("Ingress content error: {0}")]
    IngressContentError(#[from] IngressContentError),
    #[error("Publishing error: {0}")]
    PublishingError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Query error: {0}")]
    QueryError(String),
    #[error("RabbitMQ error: {0}")]
    RabbitMQError(#[from] RabbitMQError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_message) = match &self {
            ApiError::ProcessingError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::PublishingError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::DatabaseError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::QueryError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::IngressContentError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            ApiError::RabbitMQError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (
            status,
            Json(json!({
            "error": error_message,
                    "status": "error"
                        })),
        )
            .into_response()
    }
}
