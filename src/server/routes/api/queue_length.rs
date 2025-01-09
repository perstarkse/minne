use axum::{extract::State, http::StatusCode, response::IntoResponse};
use tracing::info;

use crate::{
    error::{ApiError, AppError},
    server::AppState,
    storage::{db::get_all_stored_items, types::job::Job},
};

pub async fn queue_length_handler(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Getting queue length");

    let queue_length = get_all_stored_items::<Job>(&state.surreal_db_client)
        .await
        .map_err(AppError::from)?
        .len();

    info!("Queue length: {}", queue_length);

    state
        .mailer
        .send_email_verification("per@starks.cloud", "1001010", &state.templates)
        .map_err(AppError::from)?;

    // Return the queue length with a 200 OK status
    Ok((StatusCode::OK, queue_length.to_string()))
}
