use async_openai::types::{ ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage, CreateChatCompletionRequestArgs};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;
use crate::models::file_info::FileInfo;
use thiserror::Error;

/// Represents a single piece of text content extracted from various sources.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TextContent {
    pub text: String,
    pub file_info: Option<FileInfo>,
    pub instructions: String,
    pub category: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LLMAnalysis {
    pub json_ld: serde_json::Value,
    pub description: String,
    pub related_category: String,
    pub instructions: String,
}

/// Error types for processing `TextContent`.
#[derive(Error, Debug)]
pub enum ProcessingError {
    #[error("LLM processing error: {0}")]
    LLMError(String),
    
    #[error("Graph DB storage error: {0}")]
    GraphDBError(String),
    
    #[error("Vector DB storage error: {0}")]
    VectorDBError(String),

    #[error("Unknown processing error")]
    Unknown,
}


impl TextContent {
    /// Creates a new `TextContent` instance.
    pub fn new(text: String, file_info: Option<FileInfo>, instructions: String, category: String) -> Self {
        Self {
            text,
            file_info,
            instructions,
            category,
        }
    }

    /// Processes the `TextContent` by sending it to an LLM, storing in a graph DB, and vector DB.
    pub async fn process(&self) -> Result<(), ProcessingError> {
        // Step 1: Send to LLM for analysis
        let analysis = self.send_to_llm().await?;
        info!("{:?}", analysis);

        // Step 2: Store analysis results in Graph DB
        // self.store_in_graph_db(&analysis).await?;

        // Step 3: Split text and store in Vector DB
        // self.store_in_vector_db().await?;

        Ok(())
    }

    /// Sends text to an LLM for analysis.
    async fn send_to_llm(&self) -> Result<LLMAnalysis, ProcessingError> {
        let client = async_openai::Client::new();

        // Define the JSON Schema for the expected response
//         let schema = json!({
//             "type": "object",
//     "properties": {
//         "json_ld": { 
//             "type": "object",
//             "properties": {
//                 "@context": { "type": "string" },
//                 "@type": { "type": "string" },
//                 "name": { "type": "string" }
//                 // Define only the essential properties
//             },
//             "required": ["@context", "@type", "name"],
//             "additionalProperties": false,
//         },
//         "description": { "type": "string" },
//         "related_category": { "type": "string" },
//         "instructions": { "type": "string" }
//     },
//     "required": ["json_ld", "description", "related_category", "instructions"],
//     "additionalProperties": false
// });
let  schema = json!({
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

        // Construct the system and user messages
        let system_message = format!(
            "You are an expert document analyzer. You will receive a document's text content, along with user instructions and a category. Your task is to provide a structured JSON-LD object representing the content, a short description of the document, how it relates to the submitted category, and any relevant instructions."
        );

        let user_message = format!(
            "Category: {}\nInstructions: {}\nContent:\n{}",
            self.category, self.instructions, self.text
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
            ProcessingError::LLMError(format!("OpenAI API request failed: {}", e.to_string()))
        })?;

        info!("{:?}", response);

        // Extract and parse the response
        for choice in response.choices {
            if let Some(content) = choice.message.content {
                let analysis: LLMAnalysis = serde_json::from_str(&content).map_err(|e| {
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

    /// Stores analysis results in a graph database.
    async fn store_in_graph_db(&self, _analysis: &LLMAnalysis) -> Result<(), ProcessingError> {
        // TODO: Implement storage logic for your specific graph database.
        // Example:
        /*
        let graph_db = GraphDB::new("http://graph-db:8080");
        graph_db.insert_analysis(analysis).await.map_err(|e| ProcessingError::GraphDBError(e.to_string()))?;
        */
        unimplemented!()
    }

    /// Splits text and stores it in a vector database.
    async fn store_in_vector_db(&self) -> Result<(), ProcessingError> {
        // TODO: Implement text splitting and vector storage logic.
        // Example:
        /*
        let chunks = text_splitter::split(&self.text);
        let vector_db = VectorDB::new("http://vector-db:5000");
        for chunk in chunks {
            vector_db.insert(chunk).await.map_err(|e| ProcessingError::VectorDBError(e.to_string()))?;
        }
        */
        unimplemented!()
    }
}
