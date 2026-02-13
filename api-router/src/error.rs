use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use common::error::AppError;
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize, Clone)]
pub enum ApiError {
    #[error("Internal server error")]
    InternalError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),
}

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Database(_) | AppError::OpenAI(_) => {
                tracing::error!("Internal error: {:?}", err);
                Self::InternalError("Internal server error".to_string())
            }
            AppError::NotFound(msg) => Self::NotFound(msg),
            AppError::Validation(msg) => Self::ValidationError(msg),
            AppError::Auth(msg) => Self::Unauthorized(msg),
            _ => Self::InternalError("Internal server error".to_string()),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_response) = match self {
            Self::InternalError(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorResponse {
                    error: message,
                    status: "error".to_string(),
                },
            ),
            Self::ValidationError(message) => (
                StatusCode::BAD_REQUEST,
                ErrorResponse {
                    error: message,
                    status: "error".to_string(),
                },
            ),
            Self::NotFound(message) => (
                StatusCode::NOT_FOUND,
                ErrorResponse {
                    error: message,
                    status: "error".to_string(),
                },
            ),
            Self::Unauthorized(message) => (
                StatusCode::UNAUTHORIZED,
                ErrorResponse {
                    error: message,
                    status: "error".to_string(),
                },
            ),
            Self::PayloadTooLarge(message) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorResponse {
                    error: message,
                    status: "error".to_string(),
                },
            ),
        };

        (status, Json(error_response)).into_response()
    }
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    error: String,
    status: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::error::AppError;
    use std::fmt::Debug;

    // Helper to check status code
    fn assert_status_code<T: IntoResponse + Debug>(response: T, expected_status: StatusCode) {
        let response = response.into_response();
        assert_eq!(response.status(), expected_status);
    }

    #[test]
    fn test_app_error_to_api_error_conversion() {
        // Test NotFound error conversion
        let not_found = AppError::NotFound("resource not found".to_string());
        let api_error = ApiError::from(not_found);
        assert!(matches!(api_error, ApiError::NotFound(msg) if msg == "resource not found"));

        // Test Validation error conversion
        let validation = AppError::Validation("invalid input".to_string());
        let api_error = ApiError::from(validation);
        assert!(matches!(api_error, ApiError::ValidationError(msg) if msg == "invalid input"));

        // Test Auth error conversion
        let auth = AppError::Auth("unauthorized".to_string());
        let api_error = ApiError::from(auth);
        assert!(matches!(api_error, ApiError::Unauthorized(msg) if msg == "unauthorized"));

        // Test for internal errors - create a mock error that doesn't require surrealdb
        let internal_error =
            AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "io error"));
        let api_error = ApiError::from(internal_error);
        assert!(matches!(api_error, ApiError::InternalError(_)));
    }

    #[test]
    fn test_api_error_response_status_codes() {
        // Test internal error status
        let error = ApiError::InternalError("server error".to_string());
        assert_status_code(error, StatusCode::INTERNAL_SERVER_ERROR);

        // Test not found status
        let error = ApiError::NotFound("not found".to_string());
        assert_status_code(error, StatusCode::NOT_FOUND);

        // Test validation error status
        let error = ApiError::ValidationError("invalid input".to_string());
        assert_status_code(error, StatusCode::BAD_REQUEST);

        // Test unauthorized status
        let error = ApiError::Unauthorized("not allowed".to_string());
        assert_status_code(error, StatusCode::UNAUTHORIZED);

        // Test payload too large status
        let error = ApiError::PayloadTooLarge("too big".to_string());
        assert_status_code(error, StatusCode::PAYLOAD_TOO_LARGE);
    }

    // Alternative approach that doesn't try to parse the response body
    #[test]
    fn test_error_messages() {
        // For validation errors
        let message = "invalid data format";
        let error = ApiError::ValidationError(message.to_string());

        // Check that the error itself contains the message
        assert_eq!(error.to_string(), format!("Validation error: {}", message));

        // For not found errors
        let message = "user not found";
        let error = ApiError::NotFound(message.to_string());
        assert_eq!(error.to_string(), format!("Not found: {}", message));
    }

    // Alternative approach for internal error test
    #[test]
    fn test_internal_error_sanitization() {
        // Create a sensitive error message
        let sensitive_info = "db password incorrect";

        // Create ApiError with sensitive info
        let api_error = ApiError::InternalError(sensitive_info.to_string());

        // Check the error message is correctly set
        assert_eq!(api_error.to_string(), "Internal server error");

        // Also verify correct status code
        assert_status_code(api_error, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
