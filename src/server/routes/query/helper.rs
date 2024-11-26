use serde_json::json;

use crate::{error::ApiError, storage::types::knowledge_entity::KnowledgeEntity};

use super::{
    prompt::{get_query_response_schema, QUERY_SYSTEM_PROMPT},
    LLMResponseFormat,
};

pub fn format_entities_json(entities: &[KnowledgeEntity]) -> serde_json::Value {
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

pub fn create_user_message(entities_json: &serde_json::Value, query: &str) -> String {
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
) -> Result<async_openai::types::CreateChatCompletionRequest, ApiError> {
    let response_format = async_openai::types::ResponseFormat::JsonSchema {
        json_schema: async_openai::types::ResponseFormatJsonSchema {
            description: Some("Query answering AI".into()),
            name: "query_answering_with_uuids".into(),
            schema: Some(get_query_response_schema()),
            strict: Some(true),
        },
    };

    async_openai::types::CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .temperature(0.2)
        .max_tokens(3048u32)
        .messages([
            async_openai::types::ChatCompletionRequestSystemMessage::from(QUERY_SYSTEM_PROMPT)
                .into(),
            async_openai::types::ChatCompletionRequestUserMessage::from(user_message).into(),
        ])
        .response_format(response_format)
        .build()
        .map_err(|e| ApiError::QueryError(e.to_string()))
}

pub async fn process_llm_response(
    response: async_openai::types::CreateChatCompletionResponse,
) -> Result<LLMResponseFormat, ApiError> {
    response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .ok_or(ApiError::QueryError(
            "No content found in LLM response".into(),
        ))
        .and_then(|content| {
            serde_json::from_str::<LLMResponseFormat>(content).map_err(|e| {
                ApiError::QueryError(format!("Failed to parse LLM response into analysis: {}", e))
            })
        })
}
