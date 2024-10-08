use async_openai::types::{ ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage, CreateChatCompletionRequestArgs};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;
use crate::{models::file_info::FileInfo, utils::llm::create_json_ld};
use thiserror::Error;

/// Represents a single piece of text content extracted from various sources.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TextContent {
    pub text: String,
    pub file_info: Option<FileInfo>,
    pub instructions: String,
    pub category: String,
}

/// A struct representing a knowledge source in the graph database.
#[derive(Deserialize, Debug, Serialize)]
pub struct KnowledgeSource {
    pub id: String,
    pub title: String,
    pub description: String,
    pub relationships: Vec<Relationship>,
}

/// A struct representing a relationship between knowledge sources.
#[derive(Deserialize, Serialize, Debug)]
pub struct Relationship {
    #[serde(rename = "type")]
    pub type_: String,
    pub target: String,
}

/// A struct representing the result of an LLM analysis.
#[derive(Deserialize, Debug,Serialize)]
pub struct AnalysisResult {
    pub knowledge_sources: Vec<KnowledgeSource>,
    pub category: String,
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
        let analysis = create_json_ld(&self.category, &self.instructions, &self.text).await?;
        info!("{:?}", analysis);

        // Step 2: Store analysis results in Graph DB
        // self.store_in_graph_db(&analysis).await?;

        // Step 3: Split text and store in Vector DB
        // self.store_in_vector_db().await?;

        Ok(())
    }

    /// Stores analysis results in a graph database.
    async fn store_in_graph_db(&self, _analysis: &AnalysisResult) -> Result<(), ProcessingError> {
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
