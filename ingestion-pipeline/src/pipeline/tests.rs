use std::sync::Arc;

use crate::pipeline::context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk};
use anyhow::{self, Context};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            ingestion_payload::IngestionPayload,
            ingestion_task::{IngestionTask, TaskState},
            knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
            knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk,
            text_content::TextContent,
        },
    },
};
use retrieval_pipeline::{RetrievedChunk, RetrievedEntity};
use tokio::sync::Mutex;
use super::{
    config::{IngestionConfig, IngestionTuning},
    enrichment_result::LLMEnrichmentResult,
    services::PipelineServices,
    test_support::{
        count_chunks_for_source, count_entities_for_source, count_relationships_for_source,
        persist, sample_artifacts, setup_db,
    },
    IngestionPipeline,
};

pub(crate) struct MockServices {
    text_content: TextContent,
    similar_entities: Vec<RetrievedEntity>,
    analysis: LLMEnrichmentResult,
    chunk_embedding: Vec<f32>,
    graph_entities: Vec<EmbeddedKnowledgeEntity>,
    graph_relationships: Vec<KnowledgeRelationship>,
    calls: Mutex<Vec<&'static str>>,
}

impl MockServices {
    pub(crate) fn new(user_id: &str) -> Self {
        const TEST_EMBEDDING_DIM: usize = 1536;
        let text_content = TextContent::new(
            "Example document for ingestion pipeline.".into(),
            Some("light context".into()),
            "notes".into(),
            None,
            None,
            user_id.into(),
        );
        let retrieved_entity = KnowledgeEntity::new(
            text_content.id.clone(),
            "Existing Entity".into(),
            "Previously known context".into(),
            KnowledgeEntityType::Document,
            None,
            user_id.into(),
        );

        let retrieved_chunk = TextChunk::new(
            retrieved_entity.source_id.clone(),
            "existing chunk".into(),
            user_id.into(),
        );

        let analysis = LLMEnrichmentResult {
            knowledge_entities: Vec::new(),
            relationships: Vec::new(),
        };

        let graph_entity = KnowledgeEntity::new(
            text_content.id.clone(),
            "Generated Entity".into(),
            "Entity from enrichment".into(),
            KnowledgeEntityType::Idea,
            None,
            user_id.into(),
        );
        let graph_relationship = KnowledgeRelationship::new(
            graph_entity.id.clone(),
            graph_entity.id.clone(),
            user_id.into(),
            text_content.id.clone(),
            "related_to".into(),
        );

        Self {
            text_content,
            similar_entities: vec![RetrievedEntity {
                entity: retrieved_entity,
                score: 0.8,
                chunks: std::sync::Arc::new(vec![RetrievedChunk {
                    chunk: std::sync::Arc::new(retrieved_chunk),
                    score: 0.7,
                }]),
            }],
            analysis,
            chunk_embedding: vec![0.3; TEST_EMBEDDING_DIM],
            graph_entities: vec![EmbeddedKnowledgeEntity {
                entity: graph_entity,
                embedding: vec![0.2; TEST_EMBEDDING_DIM],
            }],
            graph_relationships: vec![graph_relationship],
            calls: Mutex::new(Vec::new()),
        }
    }

    async fn record(&self, stage: &'static str) {
        self.calls.lock().await.push(stage);
    }
}

#[async_trait]
impl PipelineServices for MockServices {
    async fn prepare_text_content(
        &self,
        _payload: IngestionPayload,
    ) -> Result<TextContent, AppError> {
        self.record("prepare").await;
        Ok(self.text_content.clone())
    }

    async fn retrieve_similar_entities(
        &self,
        _content: &TextContent,
    ) -> Result<Vec<RetrievedEntity>, AppError> {
        self.record("retrieve").await;
        Ok(self.similar_entities.clone())
    }

    async fn run_enrichment(
        &self,
        _content: &TextContent,
        _similar_entities: &[RetrievedEntity],
    ) -> Result<LLMEnrichmentResult, AppError> {
        self.record("enrich").await;
        Ok(self.analysis.clone())
    }

    async fn convert_analysis(
        &self,
        content: &TextContent,
        _analysis: &LLMEnrichmentResult,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        self.record("convert").await;
        let entities = self
            .graph_entities
            .iter()
            .map(|embedded| {
                let mut embedded = embedded.clone();
                embedded.entity.source_id = content.id.clone();
                embedded
            })
            .collect();
        let relationships = self
            .graph_relationships
            .iter()
            .map(|relationship| {
                let mut relationship = relationship.clone();
                relationship.metadata.source_id = content.id.clone();
                relationship
            })
            .collect();
        Ok((entities, relationships))
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        _range: std::ops::Range<usize>,
        _overlap_tokens: usize,
    ) -> Result<Vec<EmbeddedTextChunk>, AppError> {
        self.record("chunk").await;
        Ok(vec![EmbeddedTextChunk {
            chunk: TextChunk::new(
                content.id.clone(),
                "chunk from mock services".into(),
                content.user_id.clone(),
            ),
            embedding: self.chunk_embedding.clone(),
        }])
    }
}

struct FailingServices {
    inner: MockServices,
}

struct ValidationServices;

#[async_trait]
impl PipelineServices for FailingServices {
    async fn prepare_text_content(
        &self,
        payload: IngestionPayload,
    ) -> Result<TextContent, AppError> {
        self.inner.prepare_text_content(payload).await
    }

    async fn retrieve_similar_entities(
        &self,
        content: &TextContent,
    ) -> Result<Vec<RetrievedEntity>, AppError> {
        self.inner.retrieve_similar_entities(content).await
    }

    async fn run_enrichment(
        &self,
        _content: &TextContent,
        _similar_entities: &[RetrievedEntity],
    ) -> Result<LLMEnrichmentResult, AppError> {
        Err(AppError::Processing("mock enrichment failure".to_string()))
    }

    async fn convert_analysis(
        &self,
        content: &TextContent,
        analysis: &LLMEnrichmentResult,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        self.inner
            .convert_analysis(content, analysis)
            .await
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        token_range: std::ops::Range<usize>,
        overlap_tokens: usize,
    ) -> Result<Vec<EmbeddedTextChunk>, AppError> {
        self.inner
            .prepare_chunks(content, token_range, overlap_tokens)
            .await
    }
}

#[async_trait]
impl PipelineServices for ValidationServices {
    async fn prepare_text_content(
        &self,
        _payload: IngestionPayload,
    ) -> Result<TextContent, AppError> {
        Err(AppError::Validation("unsupported".to_string()))
    }

    async fn retrieve_similar_entities(
        &self,
        _content: &TextContent,
    ) -> Result<Vec<RetrievedEntity>, AppError> {
        unreachable!("retrieve_similar_entities should not be called after validation failure")
    }

    async fn run_enrichment(
        &self,
        _content: &TextContent,
        _similar_entities: &[RetrievedEntity],
    ) -> Result<LLMEnrichmentResult, AppError> {
        unreachable!("run_enrichment should not be called after validation failure")
    }

    async fn convert_analysis(
        &self,
        _content: &TextContent,
        _analysis: &LLMEnrichmentResult,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        unreachable!("convert_analysis should not be called after validation failure")
    }

    async fn prepare_chunks(
        &self,
        _content: &TextContent,
        _token_range: std::ops::Range<usize>,
        _overlap_tokens: usize,
    ) -> Result<Vec<EmbeddedTextChunk>, AppError> {
        unreachable!("prepare_chunks should not be called after validation failure")
    }
}

pub(crate) fn pipeline_config() -> IngestionConfig {
    IngestionConfig {
        tuning: IngestionTuning {
            chunk_min_tokens: 4,
            chunk_max_tokens: 64,
            ..IngestionTuning::default()
        },
        chunk_only: false,
    }
}

pub(crate) async fn reserve_task(
    db: &SurrealDbClient,
    worker_id: &str,
    payload: IngestionPayload,
    user_id: &str,
) -> anyhow::Result<IngestionTask> {
    let task = IngestionTask::create_and_add_to_db(payload, user_id, db).await?;
    let lease = task.lease_duration();
    let claimed = IngestionTask::claim_next_ready(db, worker_id, Utc::now(), lease)
        .await?
        .context("task claimed")?;
    Ok(claimed)
}

#[tokio::test]
async fn retry_delay_grows_exponentially_and_caps() -> anyhow::Result<()> {
    use std::time::Duration;

    let db = setup_db().await?;
    let services: Arc<dyn PipelineServices> = Arc::new(MockServices::new("user-delay"));
    let pipeline = IngestionPipeline::with_services(Arc::new(db), pipeline_config(), services)?;

    // Defaults: base = 30s, cap exponent = 5, max = 900s.
    assert_eq!(pipeline.retry_delay(0), Duration::from_secs(30));
    assert_eq!(pipeline.retry_delay(1), Duration::from_secs(30));
    assert_eq!(pipeline.retry_delay(2), Duration::from_secs(60));
    assert_eq!(pipeline.retry_delay(3), Duration::from_secs(120));
    // Beyond the cap exponent the delay clamps at the configured maximum.
    assert_eq!(pipeline.retry_delay(7), Duration::from_secs(900));
    Ok(())
}

#[tokio::test]
async fn process_task_skips_pipeline_when_artifacts_already_persisted() -> anyhow::Result<()> {
    let db = setup_db().await?;
    let worker_id = "worker-persisted-skip";
    let user_id = "user-skip";
    let services = Arc::new(FailingServices {
        inner: MockServices::new(user_id),
    });
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services)?;

    let task = reserve_task(
        &db,
        worker_id,
        IngestionPayload::Text {
            text: "Already persisted payload".into(),
            context: "Context".into(),
            category: "notes".into(),
            user_id: user_id.into(),
        },
        user_id,
    )
    .await?;

    persist(&db, sample_artifacts(&task.id, user_id)).await?;

    pipeline.process_task(task.clone()).await?;

    let stored_task: IngestionTask = db.get_item(&task.id).await?.context("task present")?;
    assert_eq!(stored_task.state, TaskState::Succeeded);

    Ok(())
}

#[tokio::test]
async fn ingestion_pipeline_happy_path_persists_artifacts() -> anyhow::Result<()> {
    let db = setup_db().await?;
    let worker_id = "worker-happy";
    let user_id = "user-123";
    let services = Arc::new(MockServices::new(user_id));
    let services_clone: Arc<dyn PipelineServices> = Arc::<MockServices>::clone(&services);
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services_clone)?;

    let task = reserve_task(
        &db,
        worker_id,
        IngestionPayload::Text {
            text: "Example payload".into(),
            context: "Context".into(),
            category: "notes".into(),
            user_id: user_id.into(),
        },
        user_id,
    )
    .await?;

    pipeline.process_task(task.clone()).await?;

    let stored_task: IngestionTask = db.get_item(&task.id).await?.context("task present")?;
    assert_eq!(stored_task.state, TaskState::Succeeded);

    let text_content: TextContent = db.get_item(&task.id).await?.context("text content")?;
    assert_eq!(
        text_content.id, task.id,
        "ingested text_content id should equal the ingestion task id"
    );
    assert_eq!(count_chunks_for_source(&db, &task.id).await?, 1);
    assert_eq!(count_entities_for_source(&db, &task.id).await?, 1);
    assert_eq!(
        count_relationships_for_source(&db, &task.id).await?,
        1,
        "graph relationships should be persisted"
    );

    let call_log = services.calls.lock().await.clone();
    assert!(
        call_log.len() >= 5,
        "expected at least one chunk embedding call"
    );
    assert_eq!(
        call_log.get(0..4),
        Some(&["prepare", "retrieve", "enrich", "convert"][..])
    );
    assert!(call_log
        .get(4..)
        .is_some_and(|tail| tail.iter().all(|entry| *entry == "chunk")));
    Ok(())
}

#[tokio::test]
async fn ingestion_pipeline_chunk_only_skips_analysis() -> anyhow::Result<()> {
    let db = setup_db().await?;
    let worker_id = "worker-chunk-only";
    let user_id = "user-999";
    let services = Arc::new(MockServices::new(user_id));
    let services_clone: Arc<dyn PipelineServices> = Arc::<MockServices>::clone(&services);
    let mut config = pipeline_config();
    config.chunk_only = true;
    let pipeline = IngestionPipeline::with_services(Arc::new(db.clone()), config, services_clone)?;

    let task = reserve_task(
        &db,
        worker_id,
        IngestionPayload::Text {
            text: "Chunk only payload".into(),
            context: "Context".into(),
            category: "notes".into(),
            user_id: user_id.into(),
        },
        user_id,
    )
    .await?;

    pipeline.process_task(task.clone()).await?;

    assert_eq!(
        count_relationships_for_source(&db, &task.id).await?,
        0,
        "chunk-only ingestion should not persist relationships"
    );
    assert_eq!(
        count_chunks_for_source(&db, &task.id).await?,
        1,
        "chunk-only ingestion should still persist chunks"
    );

    let call_log = services.calls.lock().await.clone();
    assert_eq!(call_log, vec!["prepare", "chunk"]);
    Ok(())
}

#[tokio::test]
async fn produce_artifacts_returns_enriched_snapshot_without_persisting() -> anyhow::Result<()> {
    let db = setup_db().await?;
    let user_id = "user-produce";
    let services = Arc::new(MockServices::new(user_id));
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services)?;

    let payload = IngestionPayload::Text {
        text: "Produce artifacts payload".into(),
        context: "Context".into(),
        category: "notes".into(),
        user_id: user_id.into(),
    };
    let task = IngestionTask::new(payload, user_id.to_string());

    let artifacts = pipeline.produce_artifacts(&task).await?;

    assert_eq!(artifacts.text_content.user_id, user_id);
    assert_eq!(artifacts.chunks.len(), 1);
    assert_eq!(artifacts.entities.len(), 1);
    assert_eq!(artifacts.relationships.len(), 1);
    assert_eq!(count_chunks_for_source(&db, &task.id).await?, 0);
    assert_eq!(count_entities_for_source(&db, &task.id).await?, 0);

    Ok(())
}

#[tokio::test]
async fn ingestion_pipeline_failure_marks_retry() -> anyhow::Result<()> {
    let db = setup_db().await?;
    let worker_id = "worker-fail";
    let user_id = "user-456";
    let services = Arc::new(FailingServices {
        inner: MockServices::new(user_id),
    });
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services)?;

    let task = reserve_task(
        &db,
        worker_id,
        IngestionPayload::Text {
            text: "Example failure payload".into(),
            context: "Context".into(),
            category: "notes".into(),
            user_id: user_id.into(),
        },
        user_id,
    )
    .await?;

    let result = pipeline.process_task(task.clone()).await;
    assert!(
        result.is_err(),
        "failure services should bubble error from pipeline"
    );

    let stored_task: IngestionTask = db.get_item(&task.id).await?.context("task present")?;
    assert_eq!(stored_task.state, TaskState::Failed);
    assert!(
        stored_task.scheduled_at > Utc::now() - ChronoDuration::seconds(5),
        "failed task should schedule retry in the future"
    );
    Ok(())
}

#[tokio::test]
async fn ingestion_pipeline_validation_failure_dead_letters_task() -> anyhow::Result<()> {
    let db = setup_db().await?;
    let worker_id = "worker-validation";
    let user_id = "user-789";
    let services = Arc::new(ValidationServices);
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services)?;

    let task = reserve_task(
        &db,
        worker_id,
        IngestionPayload::Text {
            text: "irrelevant".into(),
            context: String::new(),
            category: "notes".into(),
            user_id: user_id.into(),
        },
        user_id,
    )
    .await?;

    let result = pipeline.process_task(task.clone()).await;
    assert!(
        result.is_err(),
        "validation failure should surface as error"
    );

    let stored_task: IngestionTask = db.get_item(&task.id).await?.context("task present")?;
    assert_eq!(stored_task.state, TaskState::DeadLetter);
    Ok(())
}
