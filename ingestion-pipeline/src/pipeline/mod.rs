mod config;
mod context;
mod enrichment_result;
mod persistence;
mod preparation;
mod services;
mod stages;
mod state;

pub use config::{IngestionConfig, IngestionTuning};
#[allow(clippy::module_name_repetitions)]
pub use context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk, PipelineArtifacts};
pub use enrichment_result::{LLMEnrichmentResult, LLMKnowledgeEntity, LLMRelationship};
#[allow(clippy::module_name_repetitions)]
pub use persistence::persist_artifacts;
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
            text_content::TextContent,
        },
    },
    utils::config::AppConfig,
};
use retrieval_pipeline::reranking::RerankerPool;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use self::{
    context::PipelineContext,
    stages::{enrich, persist, prepare_content, retrieve_related},
    state::{ready, Enriched, IngestionMachine},
};

/// Wall-clock duration of each pre-persistence pipeline stage.
struct StageTimings {
    prepare: Duration,
    retrieve: Duration,
    enrich: Duration,
}

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
            pipeline_config.tuning.embedding_query_char_limit,
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

        let pipeline_result = if self.artifacts_persisted(&processing_task.id).await? {
            info!(
                task_id = %processing_task.id,
                attempt = processing_task.attempts,
                "ingestion artifacts already persisted; skipping pipeline"
            );
            Ok(())
        } else {
            let payload = processing_task.take_content();
            self.drive_pipeline(&processing_task, payload)
                .await
                .map_err(|err| {
                    debug!(
                        task_id = %processing_task.id,
                        attempt = processing_task.attempts,
                        error = %err,
                        "ingestion pipeline failed"
                    );
                    err
                })
        };

        match pipeline_result {
            Ok(()) => self.finalize_succeeded(&processing_task).await,
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

    async fn artifacts_persisted(&self, task_id: &str) -> Result<bool, AppError> {
        Ok(self
            .db
            .get_item::<TextContent>(task_id)
            .await?
            .is_some())
    }

    async fn finalize_succeeded(&self, task: &IngestionTask) -> Result<(), AppError> {
        let tuning = &self.pipeline_config.tuning;
        let mut backoff_ms = tuning.persist_initial_backoff_ms;
        let last_attempt = tuning.persist_attempts.saturating_sub(1);

        for attempt in 0..tuning.persist_attempts {
            match task.mark_succeeded(&self.db).await {
                Ok(_) => {
                    info!(
                        task_id = %task.id,
                        attempt = task.attempts,
                        "ingestion task succeeded"
                    );
                    return Ok(());
                }
                Err(err) if attempt < last_attempt => {
                    let next_attempt = attempt.saturating_add(1);
                    warn!(
                        task_id = %task.id,
                        attempt = next_attempt,
                        error = %err,
                        "failed to mark ingestion task succeeded; retrying"
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms
                        .saturating_mul(2)
                        .min(tuning.persist_max_backoff_ms);
                }
                Err(err) => return Err(err),
            }
        }

        Err(AppError::InternalError(
            "failed to mark ingestion task succeeded after retries".into(),
        ))
    }

    fn retry_delay(&self, attempt: u32) -> Duration {
        let tuning = &self.pipeline_config.tuning;
        let capped_attempt = attempt
            .saturating_sub(1)
            .min(tuning.retry_backoff_cap_exponent);
        let multiplier = 2_u64.pow(capped_attempt);
        let delay = tuning.retry_base_delay_secs.saturating_mul(multiplier);

        Duration::from_secs(delay.min(tuning.retry_max_delay_secs))
    }

    fn duration_millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }

    /// Runs the shared `prepare → retrieve → enrich` stages, recording per-stage timings.
    ///
    /// Both the full task path ([`Self::drive_pipeline`]) and the artifact-only path
    /// ([`Self::produce_artifacts`]) share this prefix; only the terminal step differs
    /// (persist vs. return artifacts).
    async fn run_through_enrichment(
        &self,
        ctx: &mut PipelineContext<'_>,
        payload: IngestionPayload,
    ) -> Result<(IngestionMachine<(), Enriched>, StageTimings), AppError> {
        let machine = ready();

        let stage_start = Instant::now();
        let machine = prepare_content(machine, ctx, payload)
            .await
            .map_err(|err| ctx.abort(err))?;
        let prepare = stage_start.elapsed();

        let stage_start = Instant::now();
        let machine = retrieve_related(machine, ctx)
            .await
            .map_err(|err| ctx.abort(err))?;
        let retrieve = stage_start.elapsed();

        let stage_start = Instant::now();
        let machine = enrich(machine, ctx).await.map_err(|err| ctx.abort(err))?;
        let enrich = stage_start.elapsed();

        Ok((
            machine,
            StageTimings {
                prepare,
                retrieve,
                enrich,
            },
        ))
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

        let pipeline_started = Instant::now();
        let (machine, timings) = self.run_through_enrichment(&mut ctx, payload).await?;

        let stage_start = Instant::now();
        let _machine = persist(machine, &mut ctx)
            .await
            .map_err(|err| ctx.abort(err))?;
        let persist_duration = stage_start.elapsed();

        let total_duration = pipeline_started.elapsed();
        info!(
            task_id = %ctx.task_id,
            attempt = ctx.attempt,
            total_ms = Self::duration_millis(total_duration),
            prepare_ms = Self::duration_millis(timings.prepare),
            retrieve_ms = Self::duration_millis(timings.retrieve),
            enrich_ms = Self::duration_millis(timings.enrich),
            persist_ms = Self::duration_millis(persist_duration),
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

        let (_machine, _timings) = self.run_through_enrichment(&mut ctx, payload).await?;

        ctx.build_artifacts().await.map_err(|err| ctx.abort(err))
    }
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod finalize_tests {
    use std::{sync::Arc, time::Duration};

    use common::storage::types::{
        ingestion_payload::IngestionPayload,
        ingestion_task::{IngestionTask, TaskState},
    };
    use tokio::time::sleep;

    use super::{
        config::IngestionTuning,
        test_support::setup_db,
        tests::{pipeline_config, reserve_task, MockServices},
        IngestionPipeline, PipelineServices,
    };

    #[tokio::test]
    async fn finalize_succeeded_retries_mark_succeeded() -> anyhow::Result<()> {
        use anyhow::Context;
        let db = setup_db().await?;
        let worker_id = "worker-finalize-retry";
        let user_id = "user-finalize-retry";
        let services: Arc<dyn PipelineServices> = Arc::new(MockServices::new(user_id));
        let mut config = pipeline_config();
        config.tuning = IngestionTuning {
            persist_attempts: 3,
            persist_initial_backoff_ms: 10,
            persist_max_backoff_ms: 10,
            ..IngestionTuning::default()
        };
        let pipeline =
            IngestionPipeline::with_services(Arc::new(db.clone()), config, services)?;

        let task = reserve_task(
            &db,
            worker_id,
            IngestionPayload::Text {
                text: "Finalize retry payload".into(),
                context: "Context".into(),
                category: "notes".into(),
                user_id: user_id.into(),
            },
            user_id,
        )
        .await?;
        let processing = task.mark_processing(&db).await?;

        db.client
            .query(
                "UPDATE type::thing('ingestion_task', $id) SET worker_id = $wrong_worker;",
            )
            .bind(("id", processing.id.clone()))
            .bind(("wrong_worker", "wrong-worker"))
            .await?;

        let task_id = processing.id.clone();
        let db_fix = db.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(5)).await;
            let _ = db_fix
                .client
                .query(
                    "UPDATE type::thing('ingestion_task', $id) SET worker_id = $worker_id;",
                )
                .bind(("id", task_id))
                .bind(("worker_id", worker_id))
                .await;
        });

        pipeline.finalize_succeeded(&processing).await?;

        let stored: IngestionTask = db
            .get_item(&processing.id)
            .await?
            .context("task stored")?;
        assert_eq!(stored.state, TaskState::Succeeded);

        Ok(())
    }
}
