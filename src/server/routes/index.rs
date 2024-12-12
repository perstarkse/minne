use axum::{extract::State, response::Html};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde_json::json;
use surrealdb::{engine::any::Any, Surreal};
use tera::Context;
use tracing::info;

use crate::{error::ApiError, server::AppState, storage::types::user::User};

pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<Html<String>, ApiError> {
    info!("Displaying index page");

    info!("{:?}", auth.current_user);

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
