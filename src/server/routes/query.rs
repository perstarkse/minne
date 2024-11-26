pub mod helper;
pub mod prompt;

use std::sync::Arc;

use crate::{
    error::ApiError, retrieval::combined_knowledge_entity_retrieval, storage::db::SurrealDbClient,
};
use axum::{response::IntoResponse, Extension, Json};
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
struct Reference {
    #[allow(dead_code)]
    reference: String,
}

#[derive(Debug, Deserialize)]
pub struct LLMResponseFormat {
    answer: String,
    #[allow(dead_code)]
    references: Vec<Reference>,
}

pub async fn query_handler(
    Extension(db_client): Extension<Arc<SurrealDbClient>>,
    Json(query): Json<QueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", query);
    let openai_client = async_openai::Client::new();

    // Retrieve entities
    let entities =
        combined_knowledge_entity_retrieval(&db_client, &openai_client, query.query.clone())
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
    info!("{:?}", answer);

    Ok(answer.answer.into_response())
}
