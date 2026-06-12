//! Atomic persistence for ingested artifacts.
//!
//! All rows for one ingestion task are written inside a single `SurrealDB` transaction:
//! clear any prior rows for the task's `source_id`, then insert the new snapshot.
//! `SurrealDB` does not cap transaction row count; request payload size is the practical
//! limit (~4 MiB gRPC on `TiKV`). Typical single-document ingests fit comfortably.

use std::sync::Arc;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            knowledge_entity::KnowledgeEntity,
            knowledge_entity_embedding::KnowledgeEntityEmbedding,
            text_chunk::TextChunk,
            text_chunk_embedding::TextChunkEmbedding,
            text_content::TextContent,
        },
    },
};
use tokio::time::{sleep, Duration};
use tracing::warn;

use super::{
    config::IngestionTuning,
    context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk, PipelineArtifacts},
};

#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)]
pub struct PersistCounts {
    pub chunk_count: usize,
    pub entity_count: usize,
    pub relationship_count: usize,
}

/// Persists all pipeline artifacts in one database transaction.
pub async fn persist_artifacts(
    db: &SurrealDbClient,
    tuning: &IngestionTuning,
    embedding_dimensions: usize,
    artifacts: PipelineArtifacts,
) -> Result<PersistCounts, AppError> {
    let PipelineArtifacts {
        text_content,
        entities,
        relationships,
        chunks,
    } = artifacts;

    let source_id = text_content.id.clone();
    let user_id = text_content.user_id.clone();
    let chunk_count = chunks.len();
    let entity_count = entities.len();
    let relationship_count = relationships.len();

    let (entities, entity_embeddings) = prepare_entity_rows(entities, embedding_dimensions)?;
    let (chunks, chunk_embeddings) = prepare_chunk_rows(chunks, embedding_dimensions)?;

    let payload = PersistPayload {
        source_id: Arc::from(source_id),
        user_id: Arc::from(user_id),
        text_content: Arc::new(text_content),
        entities: Arc::from(entities.into_boxed_slice()),
        entity_embeddings: Arc::from(entity_embeddings.into_boxed_slice()),
        chunks: Arc::from(chunks.into_boxed_slice()),
        chunk_embeddings: Arc::from(chunk_embeddings.into_boxed_slice()),
        relationships: relationships.into(),
    };

    let mut backoff_ms = tuning.persist_initial_backoff_ms;
    let last_attempt = tuning.persist_attempts.saturating_sub(1);

    for attempt in 0..tuning.persist_attempts {
        let result = execute_persist_transaction(db, &payload).await;

        match result {
            Ok(()) => {
                return Ok(PersistCounts {
                    chunk_count,
                    entity_count,
                    relationship_count,
                });
            }
            Err(err) => {
                if is_retryable_conflict(&err) && attempt < last_attempt {
                    let next_attempt = attempt.saturating_add(1);
                    warn!(
                        attempt = next_attempt,
                        "Transient SurrealDB conflict while persisting ingestion artifacts; retrying"
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms
                        .saturating_mul(2)
                        .min(tuning.persist_max_backoff_ms);
                    continue;
                }

                return Err(err);
            }
        }
    }

    Err(AppError::InternalError(
        "Failed to persist ingestion artifacts after retries".to_string(),
    ))
}

struct PersistPayload {
    source_id: Arc<str>,
    user_id: Arc<str>,
    text_content: Arc<TextContent>,
    entities: Arc<[KnowledgeEntity]>,
    entity_embeddings: Arc<[KnowledgeEntityEmbedding]>,
    chunks: Arc<[TextChunk]>,
    chunk_embeddings: Arc<[TextChunkEmbedding]>,
    relationships:
        Arc<[common::storage::types::knowledge_relationship::KnowledgeRelationship]>,
}

async fn execute_persist_transaction(
    db: &SurrealDbClient,
    payload: &PersistPayload,
) -> Result<(), AppError> {
    #[cfg(test)]
    if test_persist_should_fail() {
        return Err(AppError::InternalError(
            "Failed to commit transaction due to a read or write conflict".into(),
        ));
    }

    let mut query = String::from("BEGIN TRANSACTION;\n");
    query.push_str(TextContent::CLEAR_INGESTED_CHILD_ROWS_SURQL);
    query.push_str(
        "DELETE type::thing('text_content', $source_id);
         UPSERT type::thing('text_content', $source_id) CONTENT $text_content;",
    );

    if !payload.entities.is_empty() {
        query.push_str("\nINSERT INTO knowledge_entity $entities;");
        query.push_str("\nINSERT INTO knowledge_entity_embedding $entity_embeddings;");
    }
    if !payload.chunks.is_empty() {
        query.push_str("\nINSERT INTO text_chunk $chunks;");
        query.push_str("\nINSERT INTO text_chunk_embedding $chunk_embeddings;");
    }
    if !payload.relationships.is_empty() {
        query.push_str(
            r#"
LET $relationships = $relationships;
FOR $relationship IN $relationships {
    LET $in_node = type::thing('knowledge_entity', $relationship.`in`);
    LET $out_node = type::thing('knowledge_entity', $relationship.out);
    RELATE $in_node->relates_to->$out_node CONTENT {
        id: type::thing('relates_to', $relationship.id),
        metadata: $relationship.metadata
    };
};"#,
        );
    }

    query.push_str("\nCOMMIT TRANSACTION;");

    let mut request = db
        .client
        .query(query)
        .bind(("source_id", Arc::clone(&payload.source_id)))
        .bind(("user_id", Arc::clone(&payload.user_id)))
        .bind(("text_content", Arc::clone(&payload.text_content)));

    if !payload.entities.is_empty() {
        request = request
            .bind(("entities", Arc::clone(&payload.entities)))
            .bind(("entity_embeddings", Arc::clone(&payload.entity_embeddings)));
    }
    if !payload.chunks.is_empty() {
        request = request
            .bind(("chunks", Arc::clone(&payload.chunks)))
            .bind(("chunk_embeddings", Arc::clone(&payload.chunk_embeddings)));
    }
    if !payload.relationships.is_empty() {
        request = request.bind(("relationships", Arc::clone(&payload.relationships)));
    }

    request
        .await
        .map_err(AppError::from)?
        .check()
        .map_err(AppError::from)?;

    Ok(())
}

fn prepare_entity_rows(
    embedded: Vec<EmbeddedKnowledgeEntity>,
    embedding_dimensions: usize,
) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeEntityEmbedding>), AppError> {
    let mut entities = Vec::with_capacity(embedded.len());
    let mut entity_embeddings = Vec::with_capacity(embedded.len());

    for item in embedded {
        KnowledgeEntityEmbedding::validate_dimension(&item.embedding, embedding_dimensions)?;
        let entity = item.entity;
        entity_embeddings.push(KnowledgeEntityEmbedding::new(
            &entity.id,
            entity.source_id.clone(),
            item.embedding,
            entity.user_id.clone(),
        ));
        entities.push(entity);
    }

    Ok((entities, entity_embeddings))
}

fn prepare_chunk_rows(
    embedded: Vec<EmbeddedTextChunk>,
    embedding_dimensions: usize,
) -> Result<(Vec<TextChunk>, Vec<TextChunkEmbedding>), AppError> {
    let mut chunks = Vec::with_capacity(embedded.len());
    let mut chunk_embeddings = Vec::with_capacity(embedded.len());

    for item in embedded {
        TextChunkEmbedding::validate_dimension(&item.embedding, embedding_dimensions)?;
        let chunk = item.chunk;
        chunk_embeddings.push(TextChunkEmbedding::new(
            &chunk.id,
            chunk.source_id.clone(),
            item.embedding,
            chunk.user_id.clone(),
        ));
        chunks.push(chunk);
    }

    Ok((chunks, chunk_embeddings))
}

fn is_retryable_conflict(error: &AppError) -> bool {
    error
        .to_string()
        .contains("Failed to commit transaction due to a read or write conflict")
}

#[cfg(test)]
static TEST_PERSIST_FAILURES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn set_test_persist_failures(count: usize) {
    TEST_PERSIST_FAILURES.store(count, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
fn test_persist_should_fail() -> bool {
    let remaining = TEST_PERSIST_FAILURES.load(std::sync::atomic::Ordering::SeqCst);
    if remaining == 0 {
        return false;
    }
    TEST_PERSIST_FAILURES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    true
}

#[cfg(test)]
mod tests {
    use common::storage::types::text_content::TextContent;

    use super::*;
    use crate::pipeline::test_support::{
        self, count_chunks_for_source, count_entities_for_source, count_relationships_for_source,
        large_artifacts, persist, sample_artifacts, setup_db, TEST_EMBEDDING_DIM,
    };

    #[tokio::test]
    async fn persist_artifacts_is_idempotent_for_same_source() -> anyhow::Result<()> {
        let db = setup_db().await?;
        let source_id = uuid::Uuid::new_v4().to_string();
        let user_id = "persist-idempotent";

        persist(&db, sample_artifacts(&source_id, user_id)).await?;
        persist(&db, sample_artifacts(&source_id, user_id)).await?;

        assert_eq!(count_chunks_for_source(&db, &source_id).await?, 1);
        assert_eq!(count_entities_for_source(&db, &source_id).await?, 1);

        Ok(())
    }

    #[tokio::test]
    async fn persist_artifacts_rejects_invalid_embedding_before_write() -> anyhow::Result<()> {
        let db = setup_db().await?;
        let source_id = uuid::Uuid::new_v4().to_string();
        let user_id = "persist-validate";
        let mut artifacts = sample_artifacts(&source_id, user_id);
        if let Some(chunk) = artifacts.chunks.first_mut() {
            chunk.embedding = vec![0.1; 2];
        }

        let result =
            persist_artifacts(&db, &test_support::tuning(), TEST_EMBEDDING_DIM, artifacts).await;
        assert!(result.is_err());

        let text: Option<TextContent> = db.get_item(&source_id).await?;
        assert!(text.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn persist_large_snapshot() -> anyhow::Result<()> {
        let db = setup_db().await?;
        let source_id = uuid::Uuid::new_v4().to_string();
        let user_id = "persist-large";
        let chunk_count = 100;
        let entity_count = 20;
        let relationship_count = 30;

        persist(
            &db,
            large_artifacts(
                &source_id,
                user_id,
                chunk_count,
                entity_count,
                relationship_count,
                TEST_EMBEDDING_DIM,
            ),
        )
        .await?;

        assert_eq!(count_chunks_for_source(&db, &source_id).await?, chunk_count);
        assert_eq!(
            count_entities_for_source(&db, &source_id).await?,
            entity_count
        );
        assert_eq!(
            count_relationships_for_source(&db, &source_id).await?,
            relationship_count
        );

        Ok(())
    }

    #[tokio::test]
    async fn persist_does_not_touch_other_source_ids() -> anyhow::Result<()> {
        let db = setup_db().await?;
        let source_a = uuid::Uuid::new_v4().to_string();
        let source_b = uuid::Uuid::new_v4().to_string();
        let user_id = "persist-isolation";

        persist(&db, large_artifacts(&source_a, user_id, 5, 3, 4, TEST_EMBEDDING_DIM)).await?;
        persist(&db, large_artifacts(&source_b, user_id, 2, 1, 1, TEST_EMBEDDING_DIM)).await?;
        persist(
            &db,
            large_artifacts(&source_a, user_id, 7, 4, 6, TEST_EMBEDDING_DIM),
        )
        .await?;

        assert_eq!(count_chunks_for_source(&db, &source_a).await?, 7);
        assert_eq!(count_entities_for_source(&db, &source_a).await?, 4);
        assert_eq!(count_relationships_for_source(&db, &source_a).await?, 6);
        assert_eq!(count_chunks_for_source(&db, &source_b).await?, 2);
        assert_eq!(count_entities_for_source(&db, &source_b).await?, 1);
        assert_eq!(count_relationships_for_source(&db, &source_b).await?, 1);

        Ok(())
    }

    #[test]
    fn is_retryable_conflict_matches_surreal_transaction_conflict() {
        let err = AppError::InternalError(
            "Failed to commit transaction due to a read or write conflict".into(),
        );
        assert!(is_retryable_conflict(&err));
    }

    #[test]
    fn is_retryable_conflict_rejects_unrelated_errors() {
        let err = AppError::Validation("invalid payload".into());
        assert!(!is_retryable_conflict(&err));
    }

    #[tokio::test]
    async fn persist_artifacts_retries_transient_conflicts() -> anyhow::Result<()> {
        set_test_persist_failures(2);

        let db = setup_db().await?;
        let source_id = uuid::Uuid::new_v4().to_string();
        let user_id = "persist-retry";
        let mut tuning = test_support::tuning();
        tuning.persist_attempts = 3;
        tuning.persist_initial_backoff_ms = 1;
        tuning.persist_max_backoff_ms = 1;

        let counts = persist_artifacts(
            &db,
            &tuning,
            TEST_EMBEDDING_DIM,
            sample_artifacts(&source_id, user_id),
        )
        .await?;

        assert_eq!(counts.chunk_count, 1);
        assert_eq!(count_chunks_for_source(&db, &source_id).await?, 1);

        Ok(())
    }
}
