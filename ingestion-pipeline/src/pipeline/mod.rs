mod config;
mod context;
mod enrichment_result;
mod preparation;
mod services;
mod stages;
mod state;

pub use config::{IngestionConfig, IngestionTuning};
pub use enrichment_result::{LLMEnrichmentResult, LLMKnowledgeEntity, LLMRelationship};
#[allow(clippy::module_name_repetitions)]
pub use services::{DefaultPipelineServices, PipelineServices};

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_openai::Client;
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        store::StorageManager,
        types::{
            ingestion_payload::IngestionPayload,
            ingestion_task::{IngestionTask, TaskErrorInfo},
        },
    },
    utils::config::AppConfig,
};
use retrieval_pipeline::reranking::RerankerPool;
use tracing::{debug, info, warn};

use self::{
    context::{PipelineArtifacts, PipelineContext},
    stages::{enrich, persist, prepare_content, retrieve_related},
    state::ready,
};

#[allow(clippy::module_name_repetitions)]
pub struct IngestionPipeline {
    db: Arc<SurrealDbClient>,
    pipeline_config: IngestionConfig,
    services: Arc<dyn PipelineServices>,
}

impl IngestionPipeline {
    pub fn new(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<Client<async_openai::config::OpenAIConfig>>,
        config: AppConfig,
        reranker_pool: Option<Arc<RerankerPool>>,
        storage: StorageManager,
        embedding_provider: Arc<common::utils::embedding::EmbeddingProvider>,
    ) -> Result<Self, AppError> {
        Self::new_with_config(
            db,
            openai_client,
            config,
            reranker_pool,
            storage,
            embedding_provider,
            IngestionConfig::default(),
        )
    }

    pub fn new_with_config(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<Client<async_openai::config::OpenAIConfig>>,
        config: AppConfig,
        reranker_pool: Option<Arc<RerankerPool>>,
        storage: StorageManager,
        embedding_provider: Arc<common::utils::embedding::EmbeddingProvider>,
        pipeline_config: IngestionConfig,
    ) -> Result<Self, AppError> {
        let services = DefaultPipelineServices::new(
            Arc::clone(&db),
            openai_client,
            config,
            reranker_pool,
            storage,
            embedding_provider,
        );

        Self::with_services(db, pipeline_config, Arc::new(services))
    }

    pub fn with_services(
        db: Arc<SurrealDbClient>,
        pipeline_config: IngestionConfig,
        services: Arc<dyn PipelineServices>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            db,
            pipeline_config,
            services,
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(
            task_id = %task.id,
            attempt = task.attempts,
            worker_id = task.worker_id.as_deref().unwrap_or("unknown-worker"),
            user_id = %task.user_id
        )
    )]
    pub async fn process_task(&self, task: IngestionTask) -> Result<(), AppError> {
        let mut processing_task = task.mark_processing(&self.db).await?;
        let payload = std::mem::replace(
            &mut processing_task.content,
            IngestionPayload::Text {
                text: String::new(),
                context: String::new(),
                category: String::new(),
                user_id: processing_task.user_id.clone(),
            },
        );

        match self
            .drive_pipeline(&processing_task, payload)
            .await
            .map_err(|err| {
                debug!(
                    task_id = %processing_task.id,
                    attempt = processing_task.attempts,
                    error = %err,
                    "ingestion pipeline failed"
                );
                err
            }) {
            Ok(()) => {
                processing_task.mark_succeeded(&self.db).await?;
                tracing::info!(
                    task_id = %processing_task.id,
                    attempt = processing_task.attempts,
                    "ingestion task succeeded"
                );
                Ok(())
            }
            Err(err) => {
                let reason = err.to_string();
                let retryable = !matches!(err, AppError::Validation(_));
                let error_info = TaskErrorInfo {
                    code: None,
                    message: reason.clone(),
                };

                if retryable && processing_task.can_retry() {
                    let delay = self.retry_delay(processing_task.attempts);
                    processing_task
                        .mark_failed(error_info, delay, &self.db)
                        .await?;
                    warn!(
                        task_id = %processing_task.id,
                        attempt = processing_task.attempts,
                        retry_in_secs = delay.as_secs(),
                        "ingestion task failed; scheduled retry"
                    );
                } else {
                    let failed_task = processing_task
                        .mark_failed(error_info.clone(), Duration::from_secs(0), &self.db)
                        .await?;
                    failed_task.mark_dead_letter(error_info, &self.db).await?;
                    warn!(
                        task_id = %failed_task.id,
                        attempt = failed_task.attempts,
                        "ingestion task failed; moved to dead letter queue"
                    );
                }

                Err(AppError::Processing(reason))
            }
        }
    }

    fn retry_delay(&self, attempt: u32) -> Duration {
        let tuning = &self.pipeline_config.tuning;
        let capped_attempt = attempt
            .saturating_sub(1)
            .min(tuning.retry_backoff_cap_exponent);
        let multiplier = 2_u64.pow(capped_attempt);
        let delay = tuning
            .retry_base_delay_secs
            .saturating_mul(multiplier);

        Duration::from_secs(delay.min(tuning.retry_max_delay_secs))
    }

    fn duration_millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }

    #[tracing::instrument(
        skip_all,
        fields(task_id = %task.id, attempt = task.attempts, user_id = %task.user_id)
    )]
    async fn drive_pipeline(
        &self,
        task: &IngestionTask,
        payload: IngestionPayload,
    ) -> Result<(), AppError> {
        let mut ctx = PipelineContext::new(
            task,
            self.db.as_ref(),
            &self.pipeline_config,
            self.services.as_ref(),
        );

        let machine = ready();

        let pipeline_started = Instant::now();

        let stage_start = Instant::now();
        let machine = prepare_content(machine, &mut ctx, payload)
            .await
            .map_err(|err| ctx.abort(err))?;
        let prepare_duration = stage_start.elapsed();

        let stage_start = Instant::now();
        let machine = retrieve_related(machine, &mut ctx)
            .await
            .map_err(|err| ctx.abort(err))?;
        let retrieve_duration = stage_start.elapsed();

        let stage_start = Instant::now();
        let machine = enrich(machine, &mut ctx)
            .await
            .map_err(|err| ctx.abort(err))?;
        let enrich_duration = stage_start.elapsed();

        let stage_start = Instant::now();
        let _machine = persist(machine, &mut ctx)
            .await
            .map_err(|err| ctx.abort(err))?;
        let persist_duration = stage_start.elapsed();

        let total_duration = pipeline_started.elapsed();
        let prepare_ms = Self::duration_millis(prepare_duration);
        let retrieve_ms = Self::duration_millis(retrieve_duration);
        let enrich_ms = Self::duration_millis(enrich_duration);
        let persist_ms = Self::duration_millis(persist_duration);
        info!(
            task_id = %ctx.task_id,
            attempt = ctx.attempt,
            total_ms = Self::duration_millis(total_duration),
            prepare_ms,
            retrieve_ms,
            enrich_ms,
            persist_ms,
            "ingestion pipeline finished"
        );

        Ok(())
    }

    /// Runs the ingestion pipeline up to (but excluding) persistence and returns the prepared artifacts.
    pub async fn produce_artifacts(
        &self,
        task: &IngestionTask,
    ) -> Result<PipelineArtifacts, AppError> {
        let payload = task.content.clone();
        let mut ctx = PipelineContext::new(
            task,
            self.db.as_ref(),
            &self.pipeline_config,
            self.services.as_ref(),
        );

        let machine = ready();
        let machine = prepare_content(machine, &mut ctx, payload)
            .await
            .map_err(|err| ctx.abort(err))?;
        let machine = retrieve_related(machine, &mut ctx)
            .await
            .map_err(|err| ctx.abort(err))?;
        let _machine = enrich(machine, &mut ctx)
            .await
            .map_err(|err| ctx.abort(err))?;

        ctx.build_artifacts().await.map_err(|err| ctx.abort(err))
    }
}

#[cfg(test)]
mod tests;
