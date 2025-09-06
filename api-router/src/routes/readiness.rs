use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::api_state::ApiState;

/// Readiness probe: returns 200 if core dependencies are ready, else 503.
pub async fn ready(State(state): State<ApiState>) -> impl IntoResponse {
    match state.db.client.query("RETURN true").await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "checks": { "db": "ok" }
            })),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "error",
                "checks": { "db": "fail" },
                "reason": e.to_string()
            })),
        ),
    }
}
