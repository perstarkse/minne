use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::Deserialize;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::HtmlError, retrieval::query_helper::get_answer_with_references, server::AppState,
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
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying search results");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let answer = get_answer_with_references(
        &state.surreal_db_client,
        &state.openai_client,
        &query.query,
        &user.id,
    )
    .await
    .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    Ok(Html(answer.content).into_response())
}
