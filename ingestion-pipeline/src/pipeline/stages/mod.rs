use std::sync::Arc;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            ingestion_payload::IngestionPayload, knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
            text_content::TextContent,
        },
    },
};
use state_machines::core::GuardError;
use tokio::time::{sleep, Duration};
use tracing::{debug, instrument, warn};

use super::{
    context::PipelineContext,
    services::PipelineServices,
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
        .map(|c| c.chars().count())
        .unwrap_or(0);

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
        .map_err(|(_, guard)| map_guard_error("prepare", guard))
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
        .map_err(|(_, guard)| map_guard_error("retrieve", guard))
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
        .map_err(|(_, guard)| map_guard_error("enrich", guard))
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
    let content = ctx.take_text_content()?;
    let analysis = ctx.take_analysis()?;

    let (entities, relationships) = ctx
        .services
        .convert_analysis(
            &content,
            &analysis,
            ctx.pipeline_config.tuning.entity_embedding_concurrency,
        )
        .await?;

    let entity_count = entities.len();
    let relationship_count = relationships.len();

    let chunk_range =
        ctx.pipeline_config.tuning.chunk_min_chars..ctx.pipeline_config.tuning.chunk_max_chars;

    let ((), chunk_count) = tokio::try_join!(
        store_graph_entities(ctx.db, &ctx.pipeline_config.tuning, entities, relationships),
        store_vector_chunks(
            ctx.db,
            ctx.services,
            ctx.task_id.as_str(),
            &content,
            chunk_range,
            &ctx.pipeline_config.tuning
        )
    )?;

    ctx.db.store_item(content).await?;
    ctx.db.rebuild_indexes().await?;

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
        .map_err(|(_, guard)| map_guard_error("persist", guard))
}

fn map_guard_error(event: &str, guard: GuardError) -> AppError {
    AppError::InternalError(format!(
        "invalid ingestion pipeline transition during {event}: {guard:?}"
    ))
}

async fn store_graph_entities(
    db: &SurrealDbClient,
    tuning: &super::config::IngestionTuning,
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
) -> Result<(), AppError> {
    const STORE_GRAPH_MUTATION: &str = r"
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
    ";

    let entities = Arc::new(entities);
    let relationships = Arc::new(relationships);

    let mut backoff_ms = tuning.graph_initial_backoff_ms;

    for attempt in 0..tuning.graph_store_attempts {
        let result = db
            .client
            .query(STORE_GRAPH_MUTATION)
            .bind(("entities", entities.clone()))
            .bind(("relationships", relationships.clone()))
            .await;

        match result {
            Ok(_) => return Ok(()),
            Err(err) => {
                if is_retryable_conflict(&err) && attempt + 1 < tuning.graph_store_attempts {
                    warn!(
                        attempt = attempt + 1,
                        "Transient SurrealDB conflict while storing graph data; retrying"
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(tuning.graph_max_backoff_ms);
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
    services: &dyn PipelineServices,
    task_id: &str,
    content: &TextContent,
    chunk_range: std::ops::Range<usize>,
    tuning: &super::config::IngestionTuning,
) -> Result<usize, AppError> {
    let prepared_chunks = services.prepare_chunks(content, chunk_range).await?;
    let chunk_count = prepared_chunks.len();

    let batch_size = tuning.chunk_insert_concurrency.max(1);
    for chunk in &prepared_chunks {
        debug!(
            task_id = %task_id,
            chunk_id = %chunk.id,
            chunk_len = chunk.chunk.chars().count(),
            "chunk persisted"
        );
    }

    for batch in prepared_chunks.chunks(batch_size) {
        store_chunk_batch(db, batch, tuning).await?;
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
    batch: &[TextChunk],
    tuning: &super::config::IngestionTuning,
) -> Result<(), AppError> {
    if batch.is_empty() {
        return Ok(());
    }

    const STORE_CHUNKS_MUTATION: &str = r"
        BEGIN TRANSACTION;
        LET $chunks = $chunks;

        FOR $chunk IN $chunks {
            CREATE type::thing('text_chunk', $chunk.id) CONTENT $chunk;
        };

        COMMIT TRANSACTION;
    ";

    let chunks = Arc::new(batch.to_vec());
    let mut backoff_ms = tuning.graph_initial_backoff_ms;

    for attempt in 0..tuning.graph_store_attempts {
        let result = db
            .client
            .query(STORE_CHUNKS_MUTATION)
            .bind(("chunks", chunks.clone()))
            .await;

        match result {
            Ok(_) => return Ok(()),
            Err(err) => {
                if is_retryable_conflict(&err) && attempt + 1 < tuning.graph_store_attempts {
                    warn!(
                        attempt = attempt + 1,
                        "Transient SurrealDB conflict while storing chunks; retrying"
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(tuning.graph_max_backoff_ms);
                    continue;
                }

                return Err(AppError::from(err));
            }
        }
    }

    Err(AppError::InternalError(
        "Failed to store text chunks after retries".to_string(),
    ))
}
