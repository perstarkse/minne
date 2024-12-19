use axum::{extract::State, response::Html};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::ApiError,
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::user::User,
};

page_data!(IndexData, {
    queue_length: u32,
});

pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<Html<String>, ApiError> {
    info!("Displaying index page");

    info!("{:?}", auth.current_user);

    let queue_length = state.rabbitmq_consumer.get_queue_length().await?;

    let output = render_template("index.html", IndexData { queue_length }, state.templates)?;

    Ok(output)
}
