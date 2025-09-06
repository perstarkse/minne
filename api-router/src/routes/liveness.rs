use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

/// Liveness probe: always returns 200 to indicate the process is running.
pub async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}
