use axum::{extract::State, http::StatusCode, response::IntoResponse};
use minijinja::context;
use tracing::{info, Instrument};

use crate::{error::ApiError, server::AppState};

pub async fn queue_length_handler(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Getting queue length");

    let queue_length = state.rabbitmq_consumer.get_queue_length().await?;

    info!("Queue length: {}", queue_length);

    state
        .mailer
        .send_email_verification("per@starks.cloud", "1001010", &state.templates)?;

    // Return the queue length with a 200 OK status
    Ok((StatusCode::OK, queue_length.to_string()))
}
