use std::sync::Arc;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        indexes::rebuild_indexes,
        types::{
            ingestion_payload::IngestionPayload, knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
        },
    },
};
use state_machines::core::GuardError;
use tokio::time::{sleep, Duration};
use tracing::{debug, instrument, warn};

use super::{
    context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk, PipelineArtifacts, PipelineContext},
    enrichment_result::LLMEnrichmentResult,
    state::{ContentPrepared, Enriched, IngestionMachine, Persisted, Ready, Retrieved},
};

const STORE_RELATIONSHIPS: &str = r"
    BEGIN TRANSACTION;
    LET $relationships = $relationships;

    FOR $relationship IN $relationships {
        LET $in_node = type::thing('knowledge_entity', $relationship.in);
        LET $out_node = type::thing('knowledge_entity', $relationship.out);
        RELATE $in_node->relates_to->$out_node CONTENT {
            id: type::thing('relates_to', $relationship.id),
            metadata: $relationship.metadata
        };
    };

    COMMIT TRANSACTION;
";

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

    debug!("Were storing chunks");
    let chunk_count = store_vector_chunks(
        ctx.db,
        ctx.task_id.as_str(),
        &chunks,
        &ctx.pipeline_config.tuning,
    )
    .await?;

    debug!("We stored chunks");
    store_graph_entities(ctx.db, &ctx.pipeline_config.tuning, entities, relationships).await?;

    debug!("Stored graph entities");

    ctx.db.store_item(text_content).await?;

    debug!("stored item");
    rebuild_indexes(ctx.db).await?;

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

async fn store_graph_entities(
    db: &SurrealDbClient,
    tuning: &super::config::IngestionTuning,
    entities: Vec<EmbeddedKnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
) -> Result<(), AppError> {
    // Persist entities with embeddings first.
    for embedded in entities {
        KnowledgeEntity::store_with_embedding(embedded.entity, embedded.embedding, db).await?;
    }

    if relationships.is_empty() {
        return Ok(());
    }

    let relationships = Arc::new(relationships);

    let mut backoff_ms = tuning.graph_initial_backoff_ms;
    let last_attempt = tuning.graph_store_attempts.saturating_sub(1);

    for attempt in 0..tuning.graph_store_attempts {
        let result = db
            .client
            .query(STORE_RELATIONSHIPS)
            .bind(("relationships", Arc::clone(&relationships)))
            .await;

        match result {
            Ok(_) => return Ok(()),
            Err(err) => {
                if is_retryable_conflict(&err) && attempt < last_attempt {
                    let next_attempt = attempt.saturating_add(1);
                    warn!(
                        attempt = next_attempt,
                        "Transient SurrealDB conflict while storing graph data; retrying"
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms
                        .saturating_mul(2)
                        .min(tuning.graph_max_backoff_ms);
                    continue;
                }

                return Err(AppError::from(err));
            }
        }
    }

    Err(AppError::InternalError(
        "Failed to store graph entities after retries".to_string(),
    ))
}

async fn store_vector_chunks(
    db: &SurrealDbClient,
    task_id: &str,
    chunks: &[EmbeddedTextChunk],
    tuning: &super::config::IngestionTuning,
) -> Result<usize, AppError> {
    let chunk_count = chunks.len();

    let batch_size = tuning.chunk_insert_concurrency.max(1);

    for batch in chunks.chunks(batch_size) {
        store_chunk_batch(db, batch, tuning, task_id).await?;
    }

    Ok(chunk_count)
}

fn is_retryable_conflict(error: &surrealdb::Error) -> bool {
    error
        .to_string()
        .contains("Failed to commit transaction due to a read or write conflict")
}

async fn store_chunk_batch(
    db: &SurrealDbClient,
    batch: &[EmbeddedTextChunk],
    _tuning: &super::config::IngestionTuning,
    task_id: &str,
) -> Result<(), AppError> {
    if batch.is_empty() {
        return Ok(());
    }

    for embedded in batch {
        TextChunk::store_with_embedding(
            embedded.chunk.to_owned(),
            embedded.embedding.to_owned(),
            db,
        )
        .await?;
        debug!(
            task_id = %task_id,
            chunk_id = %embedded.chunk.id,
            chunk_len = embedded.chunk.chunk.chars().count(),
            "chunk persisted"
        );
    }

    Ok(())
}
