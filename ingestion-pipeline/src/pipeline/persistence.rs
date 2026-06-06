//! Low-level database write mechanics for the persist stage.
//!
//! This module owns *how* ingested artifacts reach `SurrealDB` (per-item store loops,
//! the relationship transaction, and conflict retry/backoff). The persist stage in
//! [`super::stages`] owns *what* gets written and in which order.

use std::sync::Arc;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk,
        },
    },
};
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

use super::{
    config::IngestionTuning,
    context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk},
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

/// Persists chunk embeddings to the vector store.
///
/// Chunks are written serially on purpose. Concurrent/batched inserts were
/// trialed and did not reliably improve throughput; see `ingestion-pipeline/AGENTS.md`
/// for the rationale and as a candidate for future refactoring/benchmarking.
pub(super) async fn store_vector_chunks(
    db: &SurrealDbClient,
    task_id: &str,
    chunks: Vec<EmbeddedTextChunk>,
) -> Result<usize, AppError> {
    let chunk_count = chunks.len();
    for embedded in chunks {
        debug!(
            task_id = %task_id,
            chunk_id = %embedded.chunk.id,
            chunk_len = embedded.chunk.chunk.chars().count(),
            "chunk persisted"
        );
        TextChunk::store_with_embedding(embedded.chunk, embedded.embedding, db).await?;
    }

    Ok(chunk_count)
}

/// Persists knowledge entities and their relationships.
///
/// Entities are stored serially (see `store_vector_chunks` and AGENTS.md for why).
/// Relationships are written via a single transaction with bounded conflict retry.
pub(super) async fn store_graph_entities(
    db: &SurrealDbClient,
    tuning: &IngestionTuning,
    entities: Vec<EmbeddedKnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
) -> Result<(), AppError> {
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

fn is_retryable_conflict(error: &surrealdb::Error) -> bool {
    error
        .to_string()
        .contains("Failed to commit transaction due to a read or write conflict")
}
