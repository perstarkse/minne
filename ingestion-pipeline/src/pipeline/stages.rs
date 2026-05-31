//! State-machine stages of the ingestion pipeline.
//!
//! Each function advances the `IngestionMachine` by one transition
//! (`prepare → retrieve → enrich → persist`), mutating the shared
//! [`PipelineContext`]. Low-level database writes live in [`super::persistence`].

use common::{
    error::AppError,
    storage::{indexes::rebuild, types::ingestion_payload::IngestionPayload},
};
use state_machines::core::GuardError;
use tracing::{debug, instrument};

use super::{
    context::{PipelineArtifacts, PipelineContext},
    enrichment_result::LLMEnrichmentResult,
    persistence::{store_graph_entities, store_vector_chunks},
    state::{ContentPrepared, Enriched, IngestionMachine, Persisted, Ready, Retrieved},
};

#[instrument(
    level = "trace",
    skip_all,
    fields(task_id = %ctx.task_id, attempt = ctx.attempt, user_id = %ctx.task.user_id)
)]
pub async fn prepare_content(
    machine: IngestionMachine<(), Ready>,
    ctx: &mut PipelineContext<'_>,
    payload: IngestionPayload,
) -> Result<IngestionMachine<(), ContentPrepared>, AppError> {
    let text_content = ctx.services.prepare_text_content(payload).await?;

    let text_len = text_content.text.chars().count();
    let preview: String = text_content.text.chars().take(120).collect();
    let preview_clean = preview.replace('\n', " ");
    let preview_len = preview_clean.chars().count();
    let truncated = text_len > preview_len;
    let context_len = text_content
        .context
        .as_ref()
        .map_or(0, |c| c.chars().count());

    tracing::info!(
        task_id = %ctx.task_id,
        attempt = ctx.attempt,
        user_id = %text_content.user_id,
        category = %text_content.category,
        text_chars = text_len,
        context_chars = context_len,
        attachments = text_content.file_info.is_some(),
        "ingestion task input ready"
    );
    debug!(
        task_id = %ctx.task_id,
        attempt = ctx.attempt,
        preview = %preview_clean,
        preview_truncated = truncated,
        "ingestion task input preview"
    );

    ctx.text_content = Some(text_content);

    machine
        .prepare()
        .map_err(|(_, guard)| map_guard_error("prepare", &guard))
}

#[instrument(
    level = "trace",
    skip_all,
    fields(task_id = %ctx.task_id, attempt = ctx.attempt, user_id = %ctx.task.user_id)
)]
pub async fn retrieve_related(
    machine: IngestionMachine<(), ContentPrepared>,
    ctx: &mut PipelineContext<'_>,
) -> Result<IngestionMachine<(), Retrieved>, AppError> {
    if ctx.pipeline_config.chunk_only {
        return machine
            .retrieve()
            .map_err(|(_, guard)| map_guard_error("retrieve", &guard));
    }

    let content = ctx.text_content()?;
    let similar = ctx.services.retrieve_similar_entities(content).await?;

    debug!(
        task_id = %ctx.task_id,
        attempt = ctx.attempt,
        similar_count = similar.len(),
        "ingestion retrieved similar entities"
    );

    ctx.similar_entities = similar;

    machine
        .retrieve()
        .map_err(|(_, guard)| map_guard_error("retrieve", &guard))
}

#[instrument(
    level = "trace",
    skip_all,
    fields(task_id = %ctx.task_id, attempt = ctx.attempt, user_id = %ctx.task.user_id)
)]
pub async fn enrich(
    machine: IngestionMachine<(), Retrieved>,
    ctx: &mut PipelineContext<'_>,
) -> Result<IngestionMachine<(), Enriched>, AppError> {
    if ctx.pipeline_config.chunk_only {
        ctx.analysis = Some(LLMEnrichmentResult {
            knowledge_entities: Vec::new(),
            relationships: Vec::new(),
        });
        return machine
            .enrich()
            .map_err(|(_, guard)| map_guard_error("enrich", &guard));
    }

    let content = ctx.text_content()?;
    let analysis = ctx
        .services
        .run_enrichment(content, &ctx.similar_entities)
        .await?;

    debug!(
        task_id = %ctx.task_id,
        attempt = ctx.attempt,
        entity_suggestions = analysis.knowledge_entities.len(),
        relationship_suggestions = analysis.relationships.len(),
        "ingestion enrichment completed"
    );

    ctx.analysis = Some(analysis);

    machine
        .enrich()
        .map_err(|(_, guard)| map_guard_error("enrich", &guard))
}

#[instrument(
    level = "trace",
    skip_all,
    fields(task_id = %ctx.task_id, attempt = ctx.attempt, user_id = %ctx.task.user_id)
)]
pub async fn persist(
    machine: IngestionMachine<(), Enriched>,
    ctx: &mut PipelineContext<'_>,
) -> Result<IngestionMachine<(), Persisted>, AppError> {
    let PipelineArtifacts {
        text_content,
        entities,
        relationships,
        chunks,
    } = ctx.build_artifacts().await?;
    let entity_count = entities.len();
    let relationship_count = relationships.len();

    let chunk_count = store_vector_chunks(ctx.db, ctx.task_id.as_str(), &chunks).await?;
    store_graph_entities(ctx.db, &ctx.pipeline_config.tuning, entities, relationships).await?;
    ctx.db.store_item(text_content).await?;
    rebuild(ctx.db).await?;

    debug!(
        task_id = %ctx.task_id,
        attempt = ctx.attempt,
        entity_count,
        relationship_count,
        chunk_count,
        "ingestion persistence flushed to database"
    );

    machine
        .persist()
        .map_err(|(_, guard)| map_guard_error("persist", &guard))
}

fn map_guard_error(event: &str, guard: &GuardError) -> AppError {
    AppError::InternalError(format!(
        "invalid ingestion pipeline transition during {event}: {guard:?}"
    ))
}
