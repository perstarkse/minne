use axum::{extract::State, response::Html};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use minijinja::context;
use serde::Serialize;
use serde_json::json;
use surrealdb::{engine::any::Any, sql::Relation, Surreal};
use tera::Context;
// use tera::Context;
use tracing::info;

use crate::{
    error::ApiError,
    server::{routes::render_template, AppState},
    storage::types::user::User,
};

#[derive(Serialize)]
struct PageData<'a> {
    queue_length: &'a str,
}

pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<Html<String>, ApiError> {
    info!("Displaying index page");

    info!("{:?}", auth.current_user);

    let queue_length = state.rabbitmq_consumer.get_queue_length().await?;

    let output = render_template(
        "index.html",
        PageData {
            queue_length: "1000",
        },
        state.templates,
    )?;

    Ok(output)
}
