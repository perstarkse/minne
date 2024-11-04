use async_openai::types::ChatCompletionRequestSystemMessage;
use async_openai::types::ChatCompletionRequestUserMessage;
use async_openai::types::CreateChatCompletionRequestArgs;
use serde::Deserialize;
use serde::Serialize;
use tracing::debug;
use tracing::info;
use uuid::Uuid;
use crate::models::graph_entities::GraphMapper;
use crate::models::graph_entities::KnowledgeEntity;
use crate::models::graph_entities::KnowledgeEntityType;
use crate::models::graph_entities::KnowledgeRelationship;
use crate::models::text_content::ProcessingError;
use crate::surrealdb::SurrealDbClient;
use serde_json::json;

/// Represents a single knowledge entity from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMKnowledgeEntity {
    pub key: String,               // Temporary identifier
    pub name: String,
    pub description: String,
    pub entity_type: String,       // Should match KnowledgeEntityType variants
}

/// Represents a single relationship from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMRelationship {
    #[serde(rename = "type")]
    pub type_: String,              // e.g., RelatedTo, RelevantTo
    pub source: String,             // Key of the source entity
    pub target: String,             // Key of the target entity
}

/// Represents the entire graph analysis result from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMGraphAnalysisResult {
    pub knowledge_entities: Vec<LLMKnowledgeEntity>,
    pub relationships: Vec<LLMRelationship>,
}

impl LLMGraphAnalysisResult {
    pub fn to_database_entities(&self, source_id: &Uuid) -> (Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>) {
        let mut mapper = GraphMapper::new();
        
        // First pass: Create all entities and map their keys to UUIDs
        let entities: Vec<KnowledgeEntity> = self.knowledge_entities
            .iter()
            .map(|llm_entity| {
                let id = mapper.assign_id(&llm_entity.key);
                KnowledgeEntity {
                    id: id.to_string(),
                    name: llm_entity.name.clone(),
                    description: llm_entity.description.clone(),
                    entity_type: KnowledgeEntityType::from(llm_entity.entity_type.clone()),
                    source_id: source_id.to_string(),
                    metadata: None,
                }
            })
            .collect();

        // Second pass: Create relationships using mapped UUIDs
        let relationships: Vec<KnowledgeRelationship> = self.relationships
            .iter()
            .filter_map(|llm_rel| {
                let source_id = mapper.get_id(&llm_rel.source)?;
                let target_id = mapper.get_id(&llm_rel.target)?;
                
                Some(KnowledgeRelationship {
                    id: Uuid::new_v4().to_string(),
                    out: source_id.to_string(),
                    in_: target_id.to_string(),
                    relationship_type: llm_rel.type_.clone(),
                    metadata: None,
                })
            })
            .collect();

        (entities, relationships)
    }
}

/// Sends text to an LLM for analysis.
pub async fn create_json_ld(category: &str, instructions: &str, text: &str, db_client: &SurrealDbClient) -> Result<LLMGraphAnalysisResult, ProcessingError> {
    // Get the nodes from the database
    let mut result = db_client.client.query("SELECT * FROM knowledge_entity").await?;
    info!("{:?}", result.num_statements());

    let db_representation: Vec<KnowledgeEntity> = result.take(1)?;
    info!("{:?}", db_representation);
    
        let client = async_openai::Client::new();
        let schema = json!({
          "type": "object",
          "properties": {
            "knowledge_entities": {
              "type": "array",
              "items": {  
                "type": "object",
                "properties": {
                  "key": { "type": "string" },
                  "name": { "type": "string" },
                  "description": { "type": "string" },
                  "entity_type": { 
                    "type": "string",
                    "enum": ["idea", "project", "document", "page", "textsnippet"]
                  }
                },
                "required": ["key", "name", "description", "entity_type"],
                "additionalProperties": false
              }
            },
            "relationships": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "type": { 
                    "type": "string", 
                    "enum": ["RelatedTo", "RelevantTo", "SimilarTo"] 
                  },
                  "source": { "type": "string" },
                  "target": { "type": "string" }
                },
                "required": ["type", "source", "target"],
                "additionalProperties": false
              }
            }
          },
          "required": ["knowledge_entities", "relationships"],
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
        let system_message = r#"
            You are an expert document analyzer. You will receive a document's text content, along with user instructions and a category. Your task is to provide a structured JSON object representing the content in a graph format suitable for a graph database.
            
            The JSON should have the following structure:
            
            {
                "knowledge_entities": [
                    {
                        "key": "unique-key-1",
                        "name": "Entity Name",
                        "description": "A detailed description of the entity.",
                        "entity_type": "TypeOfEntity"
                    },
                    // More entities...
                ],
                "relationships": [
                    {
                        "type": "RelationshipType",
                        "source": "unique-key-1",
                        "target": "unique-key-2"
                    },
                    // More relationships...
                ]
            }
            
            Guidelines:
            1. Do NOT generate any IDs or UUIDs. Use a unique `key` for each knowledge entity.
            2. Each KnowledgeEntity should have a unique `key`, a meaningful `name`, and a descriptive `description`.
            3. Define the type of each KnowledgeEntity using the following categories: Idea, Project, Document, Page, TextSnippet.
            4. Establish relationships between entities using types like RelatedTo, RelevantTo, SimilarTo.
            5. Use the `source` key to indicate the originating entity and the `target` key to indicate the related entity"
            6. Only create relationships between existing KnowledgeEntities.
            "#; 
           

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
                let analysis: LLMGraphAnalysisResult = serde_json::from_str(&content).map_err(|e| {
                    ProcessingError::LLMError(format!(
                        "Failed to parse LLM response into analysis: {}",
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
