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
    retrieval::combined_knowledge_entity_retrieval,
    server::{
        routes::query::helper::{
            create_chat_request, create_user_message, format_entities_json, process_llm_response,
        },
        AppState,
    },
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

    let openai_client = async_openai::Client::new();

    // Retrieve entities
    let entities = combined_knowledge_entity_retrieval(
        &state.surreal_db_client,
        &openai_client,
        query.query.clone(),
    )
    .await?;

    // Format entities and create message
    let entities_json = format_entities_json(&entities);
    let user_message = create_user_message(&entities_json, &query.query);

    // Create and send request
    let request = create_chat_request(user_message)?;
    let response = openai_client
        .chat()
        .create(request)
        .await
        .map_err(|e| ApiError::QueryError(e.to_string()))?;

    // Process response
    let answer = process_llm_response(response).await?;

    let references: Vec<String> = answer
        .references
        .into_iter()
        .map(|reference| reference.reference)
        .collect();

    let output = state
        .tera
        .render(
            "search_result.html",
            &Context::from_value(json!({"result": answer.answer, "references": references}))
                .unwrap(),
        )
        .unwrap();

    Ok(output.into())
}
