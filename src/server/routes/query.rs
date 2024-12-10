pub mod helper;
pub mod prompt;

use crate::{error::ApiError, retrieval::combined_knowledge_entity_retrieval, server::AppState};
use axum::{extract::State, response::IntoResponse, Json};
use helper::{
    create_chat_request, create_user_message, format_entities_json, process_llm_response,
};
use serde::Deserialize;
use tracing::{debug, info};

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
    debug!("{:?}", user_message);

    // Create and send request
    let request = create_chat_request(user_message)?;
    let response = openai_client
        .chat()
        .create(request)
        .await
        .map_err(|e| ApiError::QueryError(e.to_string()))?;

    // Process response
    let answer = process_llm_response(response).await?;
    debug!("{:?}", answer);

    let references: Vec<String> = answer
        .references
        .into_iter()
        .map(|reference| reference.reference)
        .collect();
    info!("{:?}", references);

    Ok(
        Json(serde_json::json!({"answer": answer.answer, "references": references}))
            .into_response(),
    )
}
