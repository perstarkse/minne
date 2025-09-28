use std::{sync::Arc, time::Instant};

use chrono::Utc;
use text_splitter::TextSplitter;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

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
    utils::{config::AppConfig, embedding::generate_embedding},
};

use crate::{
    enricher::IngestionEnricher,
    types::{llm_enrichment_result::LLMEnrichmentResult, to_text_content},
};

pub struct IngestionPipeline {
    db: Arc<SurrealDbClient>,
    openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    config: AppConfig,
}

impl IngestionPipeline {
    pub async fn new(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
        config: AppConfig,
    ) -> Result<Self, AppError> {
        Ok(Self {
            db,
            openai_client,
            config,
        })
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

        let text_content =
            to_text_content(task.content, &self.db, &self.config, &self.openai_client).await?;

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
                        IngestionTaskStatus::Error {
                            message: format!("Max attempts reached: {}", e),
                        },
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
            .to_database_entities(&content.id, &content.user_id, &self.openai_client, &self.db)
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
                content.context.as_deref(),
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
        let entities = Arc::new(entities);
        let relationships = Arc::new(relationships);
        let entity_count = entities.len();
        let relationship_count = relationships.len();

        const STORE_GRAPH_MUTATION: &str = r#"
            BEGIN TRANSACTION;
            LET $entities = $entities;
            LET $relationships = $relationships;

            FOR $entity IN $entities {
                CREATE type::thing('knowledge_entity', $entity.id) CONTENT $entity;
            };

            FOR $relationship IN $relationships {
                LET $in_node = type::thing('knowledge_entity', $relationship.in);
                LET $out_node = type::thing('knowledge_entity', $relationship.out);
                RELATE $in_node->relates_to->$out_node CONTENT {
                    id: type::thing('relates_to', $relationship.id),
                    metadata: $relationship.metadata
                };
            };

            COMMIT TRANSACTION;
        "#;

        const MAX_ATTEMPTS: usize = 3;
        const INITIAL_BACKOFF_MS: u64 = 50;
        const MAX_BACKOFF_MS: u64 = 800;

        let mut backoff_ms = INITIAL_BACKOFF_MS;
        let mut success = false;

        for attempt in 0..MAX_ATTEMPTS {
            let result = self
                .db
                .client
                .query(STORE_GRAPH_MUTATION)
                .bind(("entities", entities.clone()))
                .bind(("relationships", relationships.clone()))
                .await;

            match result {
                Ok(_) => {
                    success = true;
                    break;
                }
                Err(err) => {
                    if Self::is_retryable_conflict(&err) && attempt + 1 < MAX_ATTEMPTS {
                        warn!(
                            attempt = attempt + 1,
                            "Transient SurrealDB conflict while storing graph data; retrying"
                        );
                        sleep(Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
                        continue;
                    }

                    return Err(AppError::from(err));
                }
            }
        }

        if !success {
            return Err(AppError::InternalError(
                "Failed to store graph entities after retries".to_string(),
            ));
        }

        info!(
            "Stored {} entities and {} relationships",
            entity_count, relationship_count
        );
        Ok(())
    }

    async fn store_vector_chunks(&self, content: &TextContent) -> Result<(), AppError> {
        let splitter = TextSplitter::new(500..2000);
        let chunks = splitter.chunks(&content.text);

        // Could potentially process chunks in parallel with a bounded concurrent limit
        for chunk in chunks {
            let embedding = generate_embedding(&self.openai_client, chunk, &self.db).await?;
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

    fn is_retryable_conflict(error: &surrealdb::Error) -> bool {
        error
            .to_string()
            .contains("Failed to commit transaction due to a read or write conflict")
    }
}
