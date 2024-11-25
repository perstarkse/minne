use std::time::Instant;

use text_splitter::TextSplitter;
use tracing::{debug, info};

use crate::{
    error::ProcessingError,
    storage::{
        db::{store_item, SurrealDbClient},
        types::{
            knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk, text_content::TextContent,
        },
    },
    utils::embedding::generate_embedding,
};

use super::analysis::{
    ingress_analyser::IngressAnalyzer, types::llm_analysis_result::LLMGraphAnalysisResult,
};

pub struct ContentProcessor {
    db_client: SurrealDbClient,
    openai_client: async_openai::Client<async_openai::config::OpenAIConfig>,
}

impl ContentProcessor {
    pub async fn new() -> Result<Self, ProcessingError> {
        Ok(Self {
            db_client: SurrealDbClient::new().await?,
            openai_client: async_openai::Client::new(),
        })
    }

    pub async fn process(&self, content: &TextContent) -> Result<(), ProcessingError> {
        // Store original content
        store_item(&self.db_client, content.clone()).await?;

        let now = Instant::now();
        // Process in parallel where possible
        let analysis = self.perform_semantic_analysis(content).await?;

        let end = now.elapsed();
        info!(
            "{:?} time elapsed during creation of entities and relationships",
            end
        );

        // Convert and store entities
        let (entities, relationships) = analysis
            .to_database_entities(&content.id, &self.openai_client)
            .await?;

        // Store everything
        tokio::try_join!(
            self.store_graph_entities(entities, relationships),
            self.store_vector_chunks(content),
        )?;

        self.db_client.rebuild_indexes().await?;
        Ok(())
    }

    async fn perform_semantic_analysis(
        &self,
        content: &TextContent,
    ) -> Result<LLMGraphAnalysisResult, ProcessingError> {
        let analyser = IngressAnalyzer::new(&self.db_client, &self.openai_client);
        analyser
            .analyze_content(&content.category, &content.instructions, &content.text)
            .await
    }

    async fn store_graph_entities(
        &self,
        entities: Vec<KnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
    ) -> Result<(), ProcessingError> {
        for entity in &entities {
            debug!("Storing entity: {:?}", entity);
            store_item(&self.db_client, entity.clone()).await?;
        }

        for relationship in &relationships {
            debug!("Storing relationship: {:?}", relationship);
            relationship.store_relationship(&self.db_client).await?;
        }

        info!(
            "Stored {} entities and {} relationships",
            entities.len(),
            relationships.len()
        );
        Ok(())
    }

    async fn store_vector_chunks(&self, content: &TextContent) -> Result<(), ProcessingError> {
        let splitter = TextSplitter::new(500..2000);
        let chunks = splitter.chunks(&content.text);

        // Could potentially process chunks in parallel with a bounded concurrent limit
        for chunk in chunks {
            let embedding = generate_embedding(&self.openai_client, chunk.to_string()).await?;
            let text_chunk = TextChunk::new(content.id.to_string(), chunk.to_string(), embedding);
            store_item(&self.db_client, text_chunk).await?;
        }

        Ok(())
    }
}
