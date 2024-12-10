use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;
use serde_json::json;
use tera::Context;
use tracing::info;

use crate::{
    error::ApiError,
    server::{routes::query::helper::get_answer_with_references, AppState},
};
#[derive(Deserialize)]
pub struct SearchParams {
    query: String,
}

pub async fn search_result_handler(
    State(state): State<AppState>,
    Query(query): Query<SearchParams>,
) -> Result<Html<String>, ApiError> {
    info!("Displaying search results");

    let answer =
        get_answer_with_references(&state.surreal_db_client, &state.openai_client, &query.query)
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
