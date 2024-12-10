use axum::{extract::State, response::Html};
use serde_json::json;
use tera::Context;
use tracing::info;

use crate::{error::ApiError, server::AppState};

pub async fn index_handler(State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    info!("Displaying index page");

    let queue_length = state.rabbitmq_consumer.get_queue_length().await?;

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
