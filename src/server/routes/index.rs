use axum::{extract::State, response::Html};
use serde_json::json;
use tera::Context;
use tracing::info;

use crate::{error::ApiError, server::AppState};

pub async fn index_handler(State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    info!("Displaying index page");

    // Now you can access the consumer directly from the state
    let queue_length = state.rabbitmq_consumer.queue.message_count();

    let output = state
        .tera
        .render(
            "index.html",
            &Context::from_value(json!({"adjective": "CRAYCRAY", "queue_length": queue_length}))
                .unwrap(),
        )
        .unwrap();

    Ok(output.into())
}
