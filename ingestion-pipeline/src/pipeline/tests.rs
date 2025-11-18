use std::sync::Arc;

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
use uuid::Uuid;

use super::{
    config::{IngestionConfig, IngestionTuning},
    enrichment_result::LLMEnrichmentResult,
    services::PipelineServices,
    IngestionPipeline,
};

struct MockServices {
    text_content: TextContent,
    similar_entities: Vec<RetrievedEntity>,
    analysis: LLMEnrichmentResult,
    chunk_embedding: Vec<f32>,
    graph_entities: Vec<KnowledgeEntity>,
    graph_relationships: Vec<KnowledgeRelationship>,
    calls: Mutex<Vec<&'static str>>,
}

impl MockServices {
    fn new(user_id: &str) -> Self {
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
            vec![0.1; TEST_EMBEDDING_DIM],
            user_id.into(),
        );

        let retrieved_chunk = TextChunk::new(
            retrieved_entity.source_id.clone(),
            "existing chunk".into(),
            vec![0.1; TEST_EMBEDDING_DIM],
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
            vec![0.2; TEST_EMBEDDING_DIM],
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
                chunks: vec![RetrievedChunk {
                    chunk: retrieved_chunk,
                    score: 0.7,
                }],
            }],
            analysis,
            chunk_embedding: vec![0.3; TEST_EMBEDDING_DIM],
            graph_entities: vec![graph_entity],
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
        _content: &TextContent,
        _analysis: &LLMEnrichmentResult,
        _entity_concurrency: usize,
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        self.record("convert").await;
        Ok((
            self.graph_entities.clone(),
            self.graph_relationships.clone(),
        ))
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        _range: std::ops::Range<usize>,
    ) -> Result<Vec<TextChunk>, AppError> {
        self.record("chunk").await;
        Ok(vec![TextChunk::new(
            content.id.clone(),
            "chunk from mock services".into(),
            self.chunk_embedding.clone(),
            content.user_id.clone(),
        )])
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
        entity_concurrency: usize,
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        self.inner
            .convert_analysis(content, analysis, entity_concurrency)
            .await
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        range: std::ops::Range<usize>,
    ) -> Result<Vec<TextChunk>, AppError> {
        self.inner.prepare_chunks(content, range).await
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
        _entity_concurrency: usize,
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        unreachable!("convert_analysis should not be called after validation failure")
    }

    async fn prepare_chunks(
        &self,
        _content: &TextContent,
        _range: std::ops::Range<usize>,
    ) -> Result<Vec<TextChunk>, AppError> {
        unreachable!("prepare_chunks should not be called after validation failure")
    }
}

async fn setup_db() -> SurrealDbClient {
    let namespace = "pipeline_test";
    let database = Uuid::new_v4().to_string();
    let db = SurrealDbClient::memory(namespace, &database)
        .await
        .expect("Failed to create in-memory SurrealDB");
    db.apply_migrations()
        .await
        .expect("Failed to apply migrations");
    db
}

fn pipeline_config() -> IngestionConfig {
    IngestionConfig {
        tuning: IngestionTuning {
            chunk_min_chars: 4,
            chunk_max_chars: 64,
            chunk_insert_concurrency: 4,
            entity_embedding_concurrency: 2,
            ..IngestionTuning::default()
        },
    }
}

async fn reserve_task(
    db: &SurrealDbClient,
    worker_id: &str,
    payload: IngestionPayload,
    user_id: &str,
) -> IngestionTask {
    let task = IngestionTask::create_and_add_to_db(payload, user_id.into(), db)
        .await
        .expect("task created");
    let lease = task.lease_duration();
    IngestionTask::claim_next_ready(db, worker_id, Utc::now(), lease)
        .await
        .expect("claim succeeds")
        .expect("task claimed")
}

#[tokio::test]
async fn ingestion_pipeline_happy_path_persists_entities() {
    let db = setup_db().await;
    let worker_id = "worker-happy";
    let user_id = "user-123";
    let services = Arc::new(MockServices::new(user_id));
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services.clone())
            .expect("pipeline");

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
    .await;

    pipeline
        .process_task(task.clone())
        .await
        .expect("pipeline succeeds");

    let stored_task: IngestionTask = db
        .get_item(&task.id)
        .await
        .expect("retrieve task")
        .expect("task present");
    assert_eq!(stored_task.state, TaskState::Succeeded);

    let stored_entities: Vec<KnowledgeEntity> = db
        .get_all_stored_items::<KnowledgeEntity>()
        .await
        .expect("entities stored");
    assert!(!stored_entities.is_empty(), "entities should be stored");

    let stored_chunks: Vec<TextChunk> = db
        .get_all_stored_items::<TextChunk>()
        .await
        .expect("chunks stored");
    assert!(
        !stored_chunks.is_empty(),
        "chunks should be stored for ingestion text"
    );

    let call_log = services.calls.lock().await.clone();
    assert!(
        call_log.len() >= 5,
        "expected at least one chunk embedding call"
    );
    assert_eq!(
        &call_log[0..4],
        ["prepare", "retrieve", "enrich", "convert"]
    );
    assert!(call_log[4..].iter().all(|entry| *entry == "chunk"));
}

#[tokio::test]
async fn ingestion_pipeline_failure_marks_retry() {
    let db = setup_db().await;
    let worker_id = "worker-fail";
    let user_id = "user-456";
    let services = Arc::new(FailingServices {
        inner: MockServices::new(user_id),
    });
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services)
            .expect("pipeline");

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
    .await;

    let result = pipeline.process_task(task.clone()).await;
    assert!(
        result.is_err(),
        "failure services should bubble error from pipeline"
    );

    let stored_task: IngestionTask = db
        .get_item(&task.id)
        .await
        .expect("retrieve task")
        .expect("task present");
    assert_eq!(stored_task.state, TaskState::Failed);
    assert!(
        stored_task.scheduled_at > Utc::now() - ChronoDuration::seconds(5),
        "failed task should schedule retry in the future"
    );
}

#[tokio::test]
async fn ingestion_pipeline_validation_failure_dead_letters_task() {
    let db = setup_db().await;
    let worker_id = "worker-validation";
    let user_id = "user-789";
    let services = Arc::new(ValidationServices);
    let pipeline =
        IngestionPipeline::with_services(Arc::new(db.clone()), pipeline_config(), services)
            .expect("pipeline");

    let task = reserve_task(
        &db,
        worker_id,
        IngestionPayload::Text {
            text: "irrelevant".into(),
            context: "".into(),
            category: "notes".into(),
            user_id: user_id.into(),
        },
        user_id,
    )
    .await;

    let result = pipeline.process_task(task.clone()).await;
    assert!(
        result.is_err(),
        "validation failure should surface as error"
    );

    let stored_task: IngestionTask = db
        .get_item(&task.id)
        .await
        .expect("retrieve task")
        .expect("task present");
    assert_eq!(stored_task.state, TaskState::DeadLetter);
}
