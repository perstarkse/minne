use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{ingestion_task::IngestionTask, text_content::TextContent},
    },
};
use composite_retrieval::RetrievedEntity;
use tracing::error;

use super::enrichment_result::LLMEnrichmentResult;

use super::{config::IngestionConfig, services::PipelineServices};

pub struct PipelineContext<'a> {
    pub task: &'a IngestionTask,
    pub task_id: String,
    pub attempt: u32,
    pub db: &'a SurrealDbClient,
    pub pipeline_config: &'a IngestionConfig,
    pub services: &'a dyn PipelineServices,
    pub text_content: Option<TextContent>,
    pub similar_entities: Vec<RetrievedEntity>,
    pub analysis: Option<LLMEnrichmentResult>,
}

impl<'a> PipelineContext<'a> {
    pub fn new(
        task: &'a IngestionTask,
        db: &'a SurrealDbClient,
        pipeline_config: &'a IngestionConfig,
        services: &'a dyn PipelineServices,
    ) -> Self {
        let task_id = task.id.clone();
        let attempt = task.attempts;
        Self {
            task,
            task_id,
            attempt,
            db,
            pipeline_config,
            services,
            text_content: None,
            similar_entities: Vec::new(),
            analysis: None,
        }
    }

    pub fn text_content(&self) -> Result<&TextContent, AppError> {
        self.text_content
            .as_ref()
            .ok_or_else(|| AppError::InternalError("text content expected to be available".into()))
    }

    pub fn take_text_content(&mut self) -> Result<TextContent, AppError> {
        self.text_content.take().ok_or_else(|| {
            AppError::InternalError("text content expected to be available for persistence".into())
        })
    }

    pub fn take_analysis(&mut self) -> Result<LLMEnrichmentResult, AppError> {
        self.analysis.take().ok_or_else(|| {
            AppError::InternalError("analysis expected to be available for persistence".into())
        })
    }

    pub fn abort(&mut self, err: AppError) -> AppError {
        error!(
            task_id = %self.task_id,
            attempt = self.attempt,
            error = %err,
            "ingestion pipeline aborted"
        );
        err
    }
}
