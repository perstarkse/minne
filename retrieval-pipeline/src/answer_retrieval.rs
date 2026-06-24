//! Chat answer assembly: retrieval context formatting and structured LLM request/response types.

use async_openai::{
    error::OpenAIError,
    types::chat::{
        ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
        CreateChatCompletionRequest, CreateChatCompletionRequestArgs, ResponseFormat,
        ResponseFormatJsonSchema,
    },
};
use common::storage::types::{
    message::{Message, format_history},
    system_settings::SystemSettings,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// JSON schema describing the structured chat answer (answer text + references).
fn get_query_response_schema() -> Value {
    json!({
       "type": "object",
       "properties": {
           "answer": { "type": "string" },
           "references": {
               "type": "array",
               "items": {
                   "type": "object",
                   "properties": {
                       "reference": { "type": "string" },
                   },
               "required": ["reference"],
               "additionalProperties": false,
               }
           }
       },
       "required": ["answer", "references"],
       "additionalProperties": false
    })
}

#[derive(Debug, Deserialize)]
pub struct Reference {
    pub reference: String,
}

#[derive(Debug, Deserialize)]
pub struct LLMResponseFormat {
    pub answer: String,
    pub references: Vec<Reference>,
}

impl LLMResponseFormat {
    pub fn reference_ids(&self) -> Vec<String> {
        self.references
            .iter()
            .map(|entry| entry.reference.clone())
            .collect()
    }
}

/// Convert chunk-based retrieval results to JSON format for LLM context.
pub fn chunks_to_chat_context(chunks: &[crate::RetrievedChunk]) -> Value {
    use crate::round_score;

    serde_json::json!(
        chunks
            .iter()
            .map(|chunk| {
                serde_json::json!({
                    "id": chunk.chunk.id,
                    "content": chunk.chunk.chunk,
                    "score": round_score(chunk.score),
                })
            })
            .collect::<Vec<_>>()
    )
}

pub fn create_user_message_with_history(
    context_json: &Value,
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
        context_json,
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
            schema: get_query_response_schema(),
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
