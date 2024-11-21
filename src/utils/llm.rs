use crate::{
    error::ProcessingError,
    models::graph_entities::GraphMapper,
    storage::types::{
        knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
        knowledge_relationship::KnowledgeRelationship,
    },
};
use async_openai::types::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs, CreateEmbeddingRequestArgs,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;
use tracing::{debug, info};
use uuid::Uuid;

/// Represents a single knowledge entity from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMKnowledgeEntity {
    pub key: String, // Temporary identifier
    pub name: String,
    pub description: String,
    pub entity_type: String, // Should match KnowledgeEntityType variants
}

/// Represents a single relationship from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMRelationship {
    #[serde(rename = "type")]
    pub type_: String, // e.g., RelatedTo, RelevantTo
    pub source: String, // Key of the source entity
    pub target: String, // Key of the target entity
}

/// Represents the entire graph analysis result from the LLM.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LLMGraphAnalysisResult {
    pub knowledge_entities: Vec<LLMKnowledgeEntity>,
    pub relationships: Vec<LLMRelationship>,
}

pub async fn generate_embedding(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    input: String,
) -> Result<Vec<f32>, ProcessingError> {
    let request = CreateEmbeddingRequestArgs::default()
        .model("text-embedding-3-small")
        .input(&[input])
        .build()?;

    // Send the request to OpenAI
    let response = client.embeddings().create(request).await?;

    // Extract the embedding vector
    let embedding: Vec<f32> = response
        .data
        .first()
        .ok_or_else(|| ProcessingError::EmbeddingError("No embedding data received".into()))?
        .embedding
        .clone();

    Ok(embedding)
}

impl LLMGraphAnalysisResult {
    /// Converts the LLM graph analysis result into database entities and relationships.
    /// Processes embeddings sequentially for simplicity.
    ///
    /// # Arguments
    ///
    /// * `source_id` - A UUID representing the source identifier.
    /// * `openai_client` - OpenAI client for LLM calls.
    ///
    /// # Returns
    ///
    /// * `Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), ProcessingError>` - A tuple containing vectors of `KnowledgeEntity` and `KnowledgeRelationship`.
    pub async fn to_database_entities(
        &self,
        source_id: &String,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), ProcessingError> {
        let mut mapper = GraphMapper::new();

        // Step 1: Assign unique IDs to all knowledge entities upfront
        for llm_entity in &self.knowledge_entities {
            mapper.assign_id(&llm_entity.key);
        }

        let mut entities = vec![];

        // Step 2: Process each knowledge entity sequentially
        for llm_entity in &self.knowledge_entities {
            // Retrieve the assigned ID for the current entity
            let assigned_id = mapper
                .get_id(&llm_entity.key)
                .ok_or_else(|| {
                    ProcessingError::GraphProcessingError(format!(
                        "ID not found for key: {}",
                        llm_entity.key
                    ))
                })?
                .clone();

            // Prepare the embedding input
            let embedding_input = format!(
                "name: {}, description: {}, type: {}",
                llm_entity.name, llm_entity.description, llm_entity.entity_type
            );

            // Generate embedding
            let embedding = generate_embedding(&openai_client, embedding_input).await?;

            // Construct the KnowledgeEntity with embedding
            let knowledge_entity = KnowledgeEntity {
                id: assigned_id.to_string(),
                name: llm_entity.name.clone(),
                description: llm_entity.description.clone(),
                entity_type: KnowledgeEntityType::from(llm_entity.entity_type.clone()),
                source_id: source_id.to_string(),
                metadata: None,
                embedding,
            };

            entities.push(knowledge_entity);
        }

        // Step 3: Process relationships using the pre-assigned IDs
        let relationships: Vec<KnowledgeRelationship> = self
            .relationships
            .iter()
            .filter_map(|llm_rel| {
                let source_db_id = mapper.get_or_parse_id(&llm_rel.source);
                let target_db_id = mapper.get_or_parse_id(&llm_rel.target);
                debug!("IN: {}, OUT: {}", &source_db_id, &target_db_id);

                Some(KnowledgeRelationship {
                    id: Uuid::new_v4().to_string(),
                    in_: source_db_id.to_string(),
                    out: target_db_id.to_string(),
                    relationship_type: llm_rel.type_.clone(),
                    metadata: None,
                })
            })
            .collect();

        Ok((entities, relationships))
    }
}

/// Sends text to an LLM for analysis.
pub async fn create_json_ld(
    category: &str,
    instructions: &str,
    text: &str,
    db_client: &Surreal<Client>,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<LLMGraphAnalysisResult, ProcessingError> {
    // Format the input for more cohesive comparison
    let input_text = format!(
        "content: {:?}, category: {:?}, user_instructions: {:?}",
        text, category, instructions
    );

    // Generate embedding of the input
    let input_embedding = generate_embedding(&openai_client, input_text).await?;

    let number_of_entities_to_get = 10;

    // Construct the query
    let closest_query = format!("SELECT *, vector::distance::knn() AS distance FROM knowledge_entity WHERE embedding <|{},40|> {:?} ORDER BY distance",number_of_entities_to_get, input_embedding);

    // Perform query and deserialize to struct
    let closest_entities: Vec<KnowledgeEntity> = db_client.query(closest_query).await?.take(0)?;
    #[allow(dead_code)]
    #[derive(Debug)]
    struct KnowledgeEntityToLLM {
        id: String,
        name: String,
        description: String,
    }

    info!(
        "Number of KnowledgeEntities sent as context: {}",
        closest_entities.len()
    );

    // Only keep most relevant information
    let closest_entities_to_llm: Vec<KnowledgeEntityToLLM> = closest_entities
        .clone()
        .into_iter()
        .map(|entity| KnowledgeEntityToLLM {
            id: entity.id,
            name: entity.name,
            description: entity.description,
        })
        .collect();

    debug!("{:?}", closest_entities_to_llm);

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
            You are an expert document analyzer. You will receive a document's text content, along with user instructions and a category. Your task is to provide a structured JSON object representing the content in a graph format suitable for a graph database. You will also be presented with some existing knowledge_entities from the database, do not replicate these!
            
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
                        "source": "unique-key-1 or UUID from existing database",
                        "target": "unique-key-1 or UUID from existing database"
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
            6. You will be presented with a few existing KnowledgeEntities that are similar to the current ones. They will have an existing UUID. When creating relationships to these entities, use their UUID.
            7. Only create relationships between existing KnowledgeEntities.
            8. Entities that exist already in the database should NOT be created again. If there is only a minor overlap, skip creating a new entity.
            9. A new relationship MUST include a newly created KnowledgeEntity.
            "#;

    let user_message = format!(
        "Category: {}\nInstructions: {}\nContent:\n{}\nExisting KnowledgeEntities in database:{:?}",
        category, instructions, text, closest_entities_to_llm
    );

    // Build the chat completion request
    let request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .temperature(0.2)
        .max_tokens(2048u32)
        .messages([
            ChatCompletionRequestSystemMessage::from(system_message).into(),
            ChatCompletionRequestUserMessage::from(user_message).into(),
        ])
        .response_format(response_format)
        .build()?;

    // Send the request to OpenAI
    let response = openai_client.chat().create(request).await?;

    debug!("{:?}", response);

    response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .ok_or(ProcessingError::LLMParsingError(
            "No content found in LLM response".into(),
        ))
        .and_then(|content| {
            serde_json::from_str(content).map_err(|e| {
                ProcessingError::LLMParsingError(format!(
                    "Failed to parse LLM response into analysis: {}",
                    e
                ))
            })
        })
}
