use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use uuid::Uuid;
use crate::{models::file_info::FileInfo, surrealdb::{SurrealDbClient, SurrealError}, utils::llm::create_json_ld};
use thiserror::Error;

use super::graph_entities::{KnowledgeEntity, KnowledgeRelationship};

/// Represents a single piece of text content extracted from various sources.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TextContent {
    pub id: Uuid,
    pub text: String,
    pub file_info: Option<FileInfo>,
    pub instructions: String,
    pub category: String,
}

/// Error types for processing `TextContent`.
#[derive(Error, Debug)]
pub enum ProcessingError {
    #[error("LLM processing error: {0}")]
    LLMError(String),

    #[error("SurrealDB error: {0}")]
    SurrealError(#[from] SurrealError),
    
    #[error("SurrealDb error: {0}")]
    SurrealDbError(#[from] surrealdb::Error),
    
    #[error("Graph DB storage error: {0}")]
    GraphDBError(String),
    
    #[error("Vector DB storage error: {0}")]
    VectorDBError(String),

    #[error("Unknown processing error")]
    Unknown,
}


impl TextContent {
    /// Processes the `TextContent` by sending it to an LLM, storing in a graph DB, and vector DB.
    pub async fn process(&self) -> Result<(), ProcessingError> {
        // Store TextContent
        let db_client = SurrealDbClient::new().await?;
        
        // Step 1: Send to LLM for analysis
        let analysis = create_json_ld(&self.category, &self.instructions, &self.text, &db_client).await?;
        // info!("{:#?}", &analysis);


        // Step 2: Convert LLM analysis to database entities
        let (entities, relationships) = analysis.to_database_entities(&self.id);
        
        // Step 3: Store in database
        self.store_in_graph_db(entities, relationships, &db_client).await?;
        

        // Step 4: Split text and store in Vector DB
        // self.store_in_vector_db().await?;

        Ok(())
    }

    async fn store_in_graph_db(
        &self,
        entities: Vec<KnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
        db_client: &SurrealDbClient,
    ) -> Result<(), ProcessingError> {
         for entity in entities {
            info!("{:?}", entity);
            
            let _created: Option<KnowledgeEntity> = db_client
                .client
                .create(("knowledge_entity", &entity.id.to_string()))
                .content(entity)
                .await?;

            debug!("{:?}",_created);
        }

        for relationship in relationships {
            // info!("{:?}", relationship);

            let _created: Option<KnowledgeRelationship> = db_client
                .client
                .insert(("knowledge_relationship", &relationship.id.to_string()))
                .content(relationship)
                .await?;

            debug!("{:?}",_created);
        
        }

        Ok(())
    }


    /// Splits text and stores it in a vector database.
    #[allow(dead_code)]
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
