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

page_data!(IndexData, "index/index.html", {
    queue_length: u32,
    user: Option<User>
});

pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<Html<String>, ApiError> {
    info!("Displaying index page");

    let queue_length = state.rabbitmq_consumer.get_queue_length().await?;

    // let knowledge_entities = User::get_knowledge_entities(
    //     &auth.current_user.clone().unwrap().id,
    //     &state.surreal_db_client,
    // )
    // .await?;

    // info!("{:?}", knowledge_entities);

    let output = render_template(
        IndexData::template_name(),
        IndexData {
            queue_length,
            user: auth.current_user,
        },
        state.templates,
    )?;

    Ok(output)
}
