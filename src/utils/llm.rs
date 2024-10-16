use async_openai::types::ChatCompletionRequestSystemMessage;
use async_openai::types::ChatCompletionRequestUserMessage;
use async_openai::types::CreateChatCompletionRequestArgs;
use tracing::debug;
use crate::models::text_content::ProcessingError;
use serde_json::json;
use crate::models::text_content::AnalysisResult;

/// Sends text to an LLM for analysis.
pub async fn create_json_ld(category: &str, instructions: &str, text: &str) -> Result<AnalysisResult, ProcessingError> {
        let client = async_openai::Client::new();
        let  schema = json!({
          "type": "object",
          "properties": {
            "knowledge_source": {
              "type": "object",
                "properties": {
                  "id": {"type": "string"},
                  "type": {"type": "string", "enum": ["Document", "Page", "TextSnippet"]},
                  "title": {"type": "string"},
                  "description": {"type": "string"},
                  "relationships": {
                    "type": "array",
                    "items": {
                      "type": "object",
                      "properties": {
                        "type": {"type": "string", "enum": ["RelatedTo", "RelevantTo", "SimilarTo"]},
                        "target": {"type": "string", "description": "ID of the related knowledge source"}
                      },
                      "required": ["type", "target"],
                      "additionalProperties": false,
                    }
                  }
                },
                "required": ["id", "type", "title", "description", "relationships"],
                "additionalProperties": false,
            },
            "category": {"type": "string"},
            "instructions": {"type": "string"}
          },
          "required": ["knowledge_source", "category", "instructions"],
          "additionalProperties": false
        });

        let response_format = async_openai::types::ResponseFormat::JsonSchema {
            json_schema: async_openai::types::ResponseFormatJsonSchema {
                description: Some("Structured analysis of the submitted content".into()),
                name: "content_analysis".into(),
                schema: Some(schema),
                strict: Some(true),
            },
        };

        // Construct the system and user messages
        let system_message = "You are an expert document analyzer. You will receive a document's text content, along with user instructions and a category. Your task is to provide a structured JSON-LD object representing the content, a moderately short description of the document, how it relates to the submitted category and any relevant instructions. You shall also include related objects. The goal is to insert your output into a graph database.".to_string();

        let user_message = format!(
            "Category: {}\nInstructions: {}\nContent:\n{}",
            category, instructions, text
        );

        // Build the chat completion request
        let request = CreateChatCompletionRequestArgs::default()
            .model("gpt-4o-mini") 
            .max_tokens(2048u32)
            .messages([
                ChatCompletionRequestSystemMessage::from(system_message).into(),
                ChatCompletionRequestUserMessage::from(user_message).into(),
            ])
            .response_format(response_format)
            .build().map_err(|e| ProcessingError::LLMError(e.to_string()))?;

        // Send the request to OpenAI
        let response = client.chat().create(request).await.map_err(|e| {
            ProcessingError::LLMError(format!("OpenAI API request failed: {}", e))
        })?;

        debug!("{:?}", response);

        // Extract and parse the response
        for choice in response.choices {
            if let Some(content) = choice.message.content {
                let analysis: AnalysisResult = serde_json::from_str(&content).map_err(|e| {
                    ProcessingError::LLMError(format!(
                        "Failed to parse LLM response into LLMAnalysis: {}",
                        e
                    ))
                })?;
                return Ok(analysis);
            }
        }

        Err(ProcessingError::LLMError(
            "No content found in LLM response".into(),
        ))
    }
