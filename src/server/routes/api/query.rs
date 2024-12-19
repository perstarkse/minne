use crate::{
    error::ApiError, retrieval::query_helper::get_answer_with_references, server::AppState,
    storage::types::user::User,
};
use axum::{extract::State, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use tracing::info;

#[derive(Debug, Deserialize)]
pub struct QueryInput {
    query: String,
}

pub async fn query_handler(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Json(query): Json<QueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", query);
    info!("{:?}", user);

    let answer = get_answer_with_references(
        &state.surreal_db_client,
        &state.openai_client,
        &query.query,
        &user.id,
    )
    .await?;

    Ok(
        Json(serde_json::json!({"answer": answer.content, "references": answer.references}))
            .into_response(),
    )
}
