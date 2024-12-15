use axum::{
    extract::{Query, State},
    response::Html,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::Deserialize;
use serde_json::json;
use surrealdb::{engine::any::Any, Surreal};
use tera::Context;
use tracing::info;

use crate::{
    error::ApiError,
    server::{routes::query::helper::get_answer_with_references, AppState},
    storage::types::user::User,
};
#[derive(Deserialize)]
pub struct SearchParams {
    query: String,
}

pub async fn search_result_handler(
    State(state): State<AppState>,
    Query(query): Query<SearchParams>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<Html<String>, ApiError> {
    info!("Displaying search results");

    let user_id = auth.current_user.ok_or_else(|| ApiError::AuthRequired)?.id;

    let answer = get_answer_with_references(
        &state.surreal_db_client,
        &state.openai_client,
        &query.query,
        &user_id,
    )
    .await?;

    let output = state
        .tera
        .render(
            "search_result.html",
            &Context::from_value(
                json!({"result": answer.content, "references": answer.references}),
            )
            .unwrap(),
        )
        .unwrap();

    Ok(output.into())
}
