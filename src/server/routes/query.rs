pub mod helper;
pub mod prompt;

use crate::{error::ApiError, server::AppState};
use axum::{extract::State, response::IntoResponse, Json};
use helper::get_answer_with_references;
use serde::Deserialize;
use tracing::info;

#[derive(Debug, Deserialize)]
pub struct QueryInput {
    query: String,
}

#[derive(Debug, Deserialize)]
pub struct Reference {
    #[allow(dead_code)]
    pub reference: String,
}

#[derive(Debug, Deserialize)]
pub struct LLMResponseFormat {
    pub answer: String,
    #[allow(dead_code)]
    pub references: Vec<Reference>,
}

pub async fn query_handler(
    State(state): State<AppState>,
    Json(query): Json<QueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", query);

    let answer =
        get_answer_with_references(&state.surreal_db_client, &state.openai_client, &query.query)
            .await?;

    Ok(
        Json(serde_json::json!({"answer": answer.content, "references": answer.references}))
            .into_response(),
    )
}
