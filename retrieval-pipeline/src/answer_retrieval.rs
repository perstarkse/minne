use async_openai::{
    error::OpenAIError,
    types::{
        ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
        CreateChatCompletionRequest, CreateChatCompletionRequestArgs, CreateChatCompletionResponse,
        ResponseFormat, ResponseFormatJsonSchema,
    },
};
use common::{
    error::AppError,
    storage::types::{
        message::{format_history, Message},
        system_settings::SystemSettings,
    },
};
use serde::Deserialize;
use serde_json::Value;

use super::answer_retrieval_helper::get_query_response_schema;

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

#[derive(Debug)]
pub struct Answer {
    pub content: String,
    pub references: Vec<String>,
}

pub fn create_user_message(entities_json: &Value, query: &str) -> String {
    format!(
        r"
        Context Information:
        ==================
        {entities_json}

        User Question:
        ==================
        {query}
        "
    )
}

/// Convert chunk-based retrieval results to JSON format for LLM context
pub fn chunks_to_chat_context(chunks: &[crate::RetrievedChunk]) -> Value {
    fn round_score(value: f32) -> f64 {
        (f64::from(value) * 1000.0).round() / 1000.0
    }

    serde_json::json!(chunks
        .iter()
        .map(|chunk| {
            serde_json::json!({
                "id": chunk.chunk.id,
                "content": chunk.chunk.chunk,
                "score": round_score(chunk.score),
            })
        })
        .collect::<Vec<_>>())
}

pub fn create_user_message_with_history(
    entities_json: &Value,
    history: &[Message],
    query: &str,
) -> String {
    format!(
        r"
        Chat history:
        ==================
        {}
        
        Context Information:
        ==================
        {}

        User Question:
        ==================
        {}
        ",
        format_history(history),
        entities_json,
        query
    )
}

pub fn create_chat_request(
    user_message: String,
    settings: &SystemSettings,
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
        .model(&settings.query_model)
        .messages([
            ChatCompletionRequestSystemMessage::from(settings.query_system_prompt.clone()).into(),
            ChatCompletionRequestUserMessage::from(user_message).into(),
        ])
        .response_format(response_format)
        .build()
}

pub fn process_llm_response(
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
                AppError::LLMParsing(format!("Failed to parse LLM response into analysis: {e}"))
            })
        })
}
