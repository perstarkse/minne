use std::sync::Arc;

use async_openai::error::OpenAIError;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    Json,
};
use minijinja::context;
use minijinja_autoreload::AutoReloader;
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use tokio::task::JoinError;

use crate::{
    rabbitmq::RabbitMQError, storage::types::file_info::FileError, utils::mailer::EmailError,
};

// Core internal errors
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] surrealdb::Error),
    #[error("OpenAI error: {0}")]
    OpenAI(#[from] OpenAIError),
    #[error("RabbitMQ error: {0}")]
    RabbitMQ(#[from] RabbitMQError),
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
}

// API-specific errors
#[derive(Debug, Serialize)]
pub enum ApiError {
    InternalError(String),
    ValidationError(String),
    NotFound(String),
    Unauthorized(String),
}

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Database(_) | AppError::OpenAI(_) | AppError::Email(_) => {
                tracing::error!("Internal error: {:?}", err);
                ApiError::InternalError("Internal server error".to_string())
            }
            AppError::NotFound(msg) => ApiError::NotFound(msg),
            AppError::Validation(msg) => ApiError::ValidationError(msg),
            AppError::Auth(msg) => ApiError::Unauthorized(msg),
            _ => ApiError::InternalError("Internal server error".to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::InternalError(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "error": message,
                    "status": "error"
                }),
            ),
            ApiError::ValidationError(message) => (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": message,
                    "status": "error"
                }),
            ),
            ApiError::NotFound(message) => (
                StatusCode::NOT_FOUND,
                json!({
                    "error": message,
                    "status": "error"
                }),
            ),
            ApiError::Unauthorized(message) => (
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": message,
                    "status": "error"
                }),
            ), // ... other matches
        };
        (status, Json(body)).into_response()
    }
}
#[derive(Clone)]
pub struct ErrorContext {
    #[allow(dead_code)]
    templates: Arc<AutoReloader>,
}

impl ErrorContext {
    pub fn new(templates: Arc<AutoReloader>) -> Self {
        Self { templates }
    }
}

pub enum HtmlError {
    ServerError(Arc<AutoReloader>),
    NotFound(Arc<AutoReloader>),
    Unauthorized(Arc<AutoReloader>),
    BadRequest(String, Arc<AutoReloader>),
    Template(String, Arc<AutoReloader>),
}

// Implement From<ApiError> for HtmlError
impl HtmlError {
    pub fn new(error: AppError, templates: Arc<AutoReloader>) -> Self {
        match error {
            AppError::NotFound(_msg) => HtmlError::NotFound(templates),
            AppError::Auth(_msg) => HtmlError::Unauthorized(templates),
            AppError::Validation(msg) => HtmlError::BadRequest(msg, templates),
            _ => {
                tracing::error!("Internal error: {:?}", error);
                HtmlError::ServerError(templates)
            }
        }
    }

    pub fn from_template_error(error: minijinja::Error, templates: Arc<AutoReloader>) -> Self {
        tracing::error!("Template error: {:?}", error);
        HtmlError::Template(error.to_string(), templates)
    }
}

impl IntoResponse for HtmlError {
    fn into_response(self) -> Response {
        let (status, context, templates) = match self {
            HtmlError::ServerError(templates) | HtmlError::Template(_, templates) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                context! {
                    status_code => 500,
                    title => "Internal Server Error",
                    error => "Internal Server Error",
                    description => "Something went wrong on our end."
                },
                templates,
            ),
            HtmlError::NotFound(templates) => (
                StatusCode::NOT_FOUND,
                context! {
                    status_code => 404,
                    title => "Page Not Found",
                    error => "Not Found",
                    description => "The page you're looking for doesn't exist or was removed."
                },
                templates,
            ),
            HtmlError::Unauthorized(templates) => (
                StatusCode::UNAUTHORIZED,
                context! {
                    status_code => 401,
                    title => "Unauthorized",
                    error => "Access Denied",
                    description => "You need to be logged in to access this page."
                },
                templates,
            ),
            HtmlError::BadRequest(msg, templates) => (
                StatusCode::BAD_REQUEST,
                context! {
                    status_code => 400,
                    title => "Bad Request",
                    error => "Bad Request",
                    description => msg
                },
                templates,
            ),
        };

        let html = match templates.acquire_env() {
            Ok(env) => match env.get_template("errors/error.html") {
                Ok(tmpl) => match tmpl.render(context) {
                    Ok(output) => output,
                    Err(e) => {
                        tracing::error!("Template render error: {:?}", e);
                        Self::fallback_html()
                    }
                },
                Err(e) => {
                    tracing::error!("Template get error: {:?}", e);
                    Self::fallback_html()
                }
            },
            Err(e) => {
                tracing::error!("Environment acquire error: {:?}", e);
                Self::fallback_html()
            }
        };

        (status, Html(html)).into_response()
    }
}

impl HtmlError {
    fn fallback_html() -> String {
        r#"
                     <html>
                         <body>
                             <div class="container mx-auto p-4">
                                 <h1 class="text-4xl text-error">Error</h1>
                                 <p class="mt-4">Sorry, something went wrong displaying this page.</p>
                             </div>
                         </body>
                     </html>
                     "#
        .to_string()
    }
}
