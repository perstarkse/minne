use async_openai::types::ChatCompletionRequestSystemMessage;
use async_openai::types::ChatCompletionRequestUserMessage;
use async_openai::types::CreateChatCompletionRequestArgs;
use tracing::debug;
use tracing::info;
use crate::models::text_content::ProcessingError;
use serde_json::json;
use crate::models::text_content::AnalysisResult;

/// Placeholder for your actual graph summary retrieval logic
async fn get_graph_summary() -> Result<String, ProcessingError> {
    // Implement your logic to fetch and summarize the graph database
    Ok("Current graph contains documents related to AI, Rust programming, and asynchronous systems.".into())
}

/// Sends text to an LLM for analysis with enhanced functionality.
pub async fn create_json_ld(category: &str, instructions: &str, text: &str) -> Result<AnalysisResult, ProcessingError> {
    let client = async_openai::Client::new();
    
    // Fetch the graph summary
    let graph_summary = get_graph_summary().await?;
    
    let schema = json!({
        "type": "object",
        "properties": {
            "knowledge_sources": {
                "type": "array",
                "items": {
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
                }
            },
            "category": {"type": "string"},
            "instructions": {"type": "string"}
        },
        "required": ["knowledge_sources", "category", "instructions"],
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

    // Construct examples to guide the LLM
    let system_message = format!(
        "You are an expert document analyzer. You will receive a document's text content, user instructions, a category, and a summary of the current graph database. Your task is to provide a structured JSON-LD object representing the content, a moderately short description of the document, how it relates to the submitted category and any relevant instructions. You shall also include related objects. The goal is to insert your output into a graph database. 

        Here are examples of the desired output:

        Example 1:
        {{
          \"knowledge_sources\": [
            {{
              \"id\": \"ai_neural_networks\",
              \"type\": \"Document\",
              \"title\": \"Understanding Neural Networks\",
              \"description\": \"An in-depth analysis of neural networks and their applications in machine learning.\",
              \"relationships\": [
                {{
                  \"type\": \"RelatedTo\",
                  \"target\": \"ai_machine_learning\"
                }},
                {{
                  \"type\": \"SimilarTo\",
                  \"target\": \"ai_deep_learning\"
                }}
              ]
            }}
          ],
          \"category\": \"ai\",
          \"instructions\": \"Analyze the document and relate it to existing AI knowledge.\"
        }}

        Example 2:
        {{
          \"knowledge_sources\": [
            {{
              \"id\": \"rust_async_programming\",
              \"type\": \"Document\",
              \"title\": \"Asynchronous Programming in Rust\",
              \"description\": \"A comprehensive guide to writing asynchronous code in Rust using async/await syntax.\",
              \"relationships\": [
                {{
                  \"type\": \"RelatedTo\",
                  \"target\": \"rust_concurrency\"
                }},
                {{
                  \"type\": \"SimilarTo\",
                  \"target\": \"rust_multithreading\"
                }}
              ]
            }}
          ],
          \"category\": \"rust\",
          \"instructions\": \"Incorporate the document into the Rust programming knowledge base.\"
        }}

        Please ensure the IDs follow the format <category>_<short_description> using snake_case."
    );

    let user_message = format!(
        "Graph Summary: {}\nCategory: {}\nInstructions: {}\nContent:\n{}",
        graph_summary, category, instructions, text
    );

    // Build the chat completion request
    let request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini") // Ensure this is the correct model identifier
        .max_tokens(2048u32)
        .messages([
            ChatCompletionRequestSystemMessage::from(system_message).into(),
            ChatCompletionRequestUserMessage::from(user_message).into(),
        ])
        .response_format(response_format)
        .build()
        .map_err(|e| ProcessingError::LLMError(e.to_string()))?;

    // Send the request to OpenAI
    let response = client.chat().create(request).await.map_err(|e| {
        ProcessingError::LLMError(format!("OpenAI API request failed: {}", e.to_string()))
    })?;

    debug!("{:?}", response);

    // Extract and parse the response
    for choice in response.choices {
        if let Some(content) = choice.message.content {
            let analysis: AnalysisResult = serde_json::from_str(&content).map_err(|e| {
                ProcessingError::LLMError(format!(
                    "Failed to parse LLM response into LLMAnalysis: {}",
                    e.to_string()
                ))
            })?;
            return Ok(analysis);
        }
    }

    Err(ProcessingError::LLMError(
        "No content found in LLM response".into(),
    ))
}
