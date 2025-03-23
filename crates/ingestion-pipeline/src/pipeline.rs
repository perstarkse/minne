use std::{sync::Arc, time::Instant};

use chrono::Utc;
use text_splitter::TextSplitter;
use tracing::{debug, info};

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            ingestion_task::{IngestionTask, IngestionTaskStatus, MAX_ATTEMPTS},
            knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk,
            text_content::TextContent,
        },
    },
    utils::embedding::generate_embedding,
};

use crate::{
    enricher::IngestionEnricher,
    types::{llm_enrichment_result::LLMEnrichmentResult, to_text_content},
};

pub struct IngestionPipeline {
    db: Arc<SurrealDbClient>,
    openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
}

impl IngestionPipeline {
    pub async fn new(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    ) -> Result<Self, AppError> {
        Ok(Self { db, openai_client })
    }
    pub async fn process_task(&self, task: IngestionTask) -> Result<(), AppError> {
        let current_attempts = match task.status {
            IngestionTaskStatus::InProgress { attempts, .. } => attempts + 1,
            _ => 1,
        };

        // Update status to InProgress with attempt count
        IngestionTask::update_status(
            &task.id,
            IngestionTaskStatus::InProgress {
                attempts: current_attempts,
                last_attempt: Utc::now(),
            },
            &self.db,
        )
        .await?;

        let text_content = to_text_content(task.content, &self.openai_client, &self.db).await?;

        match self.process(&text_content).await {
            Ok(_) => {
                IngestionTask::update_status(&task.id, IngestionTaskStatus::Completed, &self.db)
                    .await?;
                Ok(())
            }
            Err(e) => {
                if current_attempts >= MAX_ATTEMPTS {
                    IngestionTask::update_status(
                        &task.id,
                        IngestionTaskStatus::Error(format!("Max attempts reached: {}", e)),
                        &self.db,
                    )
                    .await?;
                }
                Err(AppError::Processing(e.to_string()))
            }
        }
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

        // Convert analysis to application objects
        let (entities, relationships) = analysis
            .to_database_entities(&content.id, &content.user_id, &self.openai_client)
            .await?;

        // Store everything
        tokio::try_join!(
            self.store_graph_entities(entities, relationships),
            self.store_vector_chunks(content),
        )?;

        // Store original content
        self.db.store_item(content.to_owned()).await?;

        self.db.rebuild_indexes().await?;
        Ok(())
    }

    async fn perform_semantic_analysis(
        &self,
        content: &TextContent,
    ) -> Result<LLMEnrichmentResult, AppError> {
        let analyser = IngestionEnricher::new(self.db.clone(), self.openai_client.clone());
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
            self.db.store_item(entity.clone()).await?;
        }

        for relationship in &relationships {
            debug!("Storing relationship: {:?}", relationship);
            relationship.store_relationship(&self.db).await?;
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
            self.db.store_item(text_chunk).await?;
        }

        Ok(())
    }
}
