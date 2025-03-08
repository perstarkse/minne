use async_openai::{
    error::OpenAIError,
    types::{
        ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
        CreateChatCompletionRequest, CreateChatCompletionRequestArgs, CreateChatCompletionResponse,
        ResponseFormat, ResponseFormatJsonSchema,
    },
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::knowledge_entity::KnowledgeEntity},
};

use crate::retrieve_entities;

use super::answer_retrieval_helper::{get_query_response_schema, QUERY_SYSTEM_PROMPT};

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

/// Orchestrates query processing and returns an answer with references
///
/// Takes a query and uses the provided clients to generate an answer with supporting references.
///
/// # Arguments
///
/// * `surreal_db_client` - Client for SurrealDB interactions
/// * `openai_client` - Client for OpenAI API calls
/// * `query` - The user's query string
/// * `user_id` - The user's id
///
/// # Returns
///
/// Returns a tuple of the answer and its references, or an API error
#[derive(Debug)]
pub struct Answer {
    pub content: String,
    pub references: Vec<String>,
}

pub async fn get_answer_with_references(
    surreal_db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    query: &str,
    user_id: &str,
) -> Result<Answer, AppError> {
    let entities = retrieve_entities(surreal_db_client, openai_client, query, user_id).await?;

    let entities_json = format_entities_json(&entities);
    debug!("{:?}", entities_json);
    let user_message = create_user_message(&entities_json, query);

    let request = create_chat_request(user_message)?;
    let response = openai_client.chat().create(request).await?;

    let llm_response = process_llm_response(response).await?;

    Ok(Answer {
        content: llm_response.answer,
        references: llm_response
            .references
            .into_iter()
            .map(|r| r.reference)
            .collect(),
    })
}

pub fn format_entities_json(entities: &[KnowledgeEntity]) -> Value {
    json!(entities
        .iter()
        .map(|entity| {
            json!({
                "KnowledgeEntity": {
                    "id": entity.id,
                    "name": entity.name,
                    "description": entity.description
                }
            })
        })
        .collect::<Vec<_>>())
}

pub fn create_user_message(entities_json: &Value, query: &str) -> String {
    format!(
        r#"
        Context Information:
        ==================
        {}

        User Question:
        ==================
        {}
        "#,
        entities_json, query
    )
}

pub fn create_chat_request(
    user_message: String,
) -> Result<CreateChatCompletionRequest, OpenAIError> {
    let response_format = ResponseFormat::JsonSchema {
        json_schema: ResponseFormatJsonSchema {
            description: Some("Query answering AI".into()),
            name: "query_answering_with_uuids".into(),
            schema: Some(get_query_response_schema()),
            strict: Some(true),
        },
    };

    CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .temperature(0.2)
        .max_tokens(3048u32)
        .messages([
            ChatCompletionRequestSystemMessage::from(QUERY_SYSTEM_PROMPT).into(),
            ChatCompletionRequestUserMessage::from(user_message).into(),
        ])
        .response_format(response_format)
        .build()
}

pub async fn process_llm_response(
    response: CreateChatCompletionResponse,
) -> Result<LLMResponseFormat, AppError> {
    response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .ok_or(AppError::LLMParsing(
            "No content found in LLM response".into(),
        ))
        .and_then(|content| {
            serde_json::from_str::<LLMResponseFormat>(content).map_err(|e| {
                AppError::LLMParsing(format!("Failed to parse LLM response into analysis: {}", e))
            })
        })
}
