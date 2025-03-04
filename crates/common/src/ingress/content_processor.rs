use std::{sync::Arc, time::Instant};

use text_splitter::TextSplitter;
use tracing::{debug, info};

use crate::{
    error::AppError,
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
    db_client: Arc<SurrealDbClient>,
    openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
}

impl ContentProcessor {
    pub async fn new(
        surreal_db_client: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            db_client: surreal_db_client,
            openai_client,
        })
    }

    pub async fn process(&self, content: &TextContent) -> Result<(), AppError> {
        let now = Instant::now();

        // Perform analyis, this step also includes retrieval
        let analysis = self.perform_semantic_analysis(content).await?;

        let end = now.elapsed();
        info!(
            "{:?} time elapsed during creation of entities and relationships",
            end
        );

        // Convert analysis to objects
        let (entities, relationships) = analysis
            .to_database_entities(&content.id, &content.user_id, &self.openai_client)
            .await?;

        // Store everything
        tokio::try_join!(
            self.store_graph_entities(entities, relationships),
            self.store_vector_chunks(content),
        )?;

        // Store original content
        store_item(&self.db_client, content.to_owned()).await?;

        self.db_client.rebuild_indexes().await?;
        Ok(())
    }

    async fn perform_semantic_analysis(
        &self,
        content: &TextContent,
    ) -> Result<LLMGraphAnalysisResult, AppError> {
        let analyser = IngressAnalyzer::new(&self.db_client, &self.openai_client);
        analyser
            .analyze_content(
                &content.category,
                &content.instructions,
                &content.text,
                &content.user_id,
            )
            .await
    }

    async fn store_graph_entities(
        &self,
        entities: Vec<KnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
    ) -> Result<(), AppError> {
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

    async fn store_vector_chunks(&self, content: &TextContent) -> Result<(), AppError> {
        let splitter = TextSplitter::new(500..2000);
        let chunks = splitter.chunks(&content.text);

        // Could potentially process chunks in parallel with a bounded concurrent limit
        for chunk in chunks {
            let embedding = generate_embedding(&self.openai_client, chunk).await?;
            let text_chunk = TextChunk::new(
                content.id.to_string(),
                chunk.to_string(),
                embedding,
                content.user_id.to_string(),
            );
            store_item(&self.db_client, text_chunk).await?;
        }

        Ok(())
    }
}
