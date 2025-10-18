mod config;
mod stages;
mod state;

pub use config::{RetrievalConfig, RetrievalTuning};

use crate::RetrievedEntity;
use async_openai::Client;
use common::{error::AppError, storage::db::SurrealDbClient};
use tracing::info;

/// Drives the retrieval pipeline from embedding through final assembly.
pub async fn run_pipeline(
    db_client: &SurrealDbClient,
    openai_client: &Client<async_openai::config::OpenAIConfig>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
) -> Result<Vec<RetrievedEntity>, AppError> {
    let machine = state::ready();
    let input_chars = input_text.chars().count();
    let input_preview: String = input_text.chars().take(120).collect();
    let input_preview_clean = input_preview.replace('\n', " ");
    let preview_len = input_preview_clean.chars().count();
    info!(
        %user_id,
        input_chars,
        preview_truncated = input_chars > preview_len,
        preview = %input_preview_clean,
        "Starting ingestion retrieval pipeline"
    );
    let mut ctx = stages::PipelineContext::new(
        db_client,
        openai_client,
        input_text.to_owned(),
        user_id.to_owned(),
        config,
    );
    let machine = stages::embed(machine, &mut ctx).await?;
    let machine = stages::collect_candidates(machine, &mut ctx).await?;
    let machine = stages::expand_graph(machine, &mut ctx).await?;
    let machine = stages::attach_chunks(machine, &mut ctx).await?;
    let results = stages::assemble(machine, &mut ctx)?;

    Ok(results)
}

#[cfg(test)]
pub async fn run_pipeline_with_embedding(
    db_client: &SurrealDbClient,
    openai_client: &Client<async_openai::config::OpenAIConfig>,
    query_embedding: Vec<f32>,
    input_text: &str,
    user_id: &str,
    config: RetrievalConfig,
) -> Result<Vec<RetrievedEntity>, AppError> {
    let machine = state::ready();
    let mut ctx = stages::PipelineContext::with_embedding(
        db_client,
        openai_client,
        query_embedding,
        input_text.to_owned(),
        user_id.to_owned(),
        config,
    );
    let machine = stages::embed(machine, &mut ctx).await?;
    let machine = stages::collect_candidates(machine, &mut ctx).await?;
    let machine = stages::expand_graph(machine, &mut ctx).await?;
    let machine = stages::attach_chunks(machine, &mut ctx).await?;
    let results = stages::assemble(machine, &mut ctx)?;

    Ok(results)
}

/// Helper exposed for tests to convert retrieved entities into downstream prompt JSON.
pub fn retrieved_entities_to_json(entities: &[RetrievedEntity]) -> serde_json::Value {
    serde_json::json!(entities
        .iter()
        .map(|entry| {
            serde_json::json!({
                "KnowledgeEntity": {
                    "id": entry.entity.id,
                    "name": entry.entity.name,
                    "description": entry.entity.description,
                    "score": round_score(entry.score),
                    "chunks": entry.chunks.iter().map(|chunk| {
                        serde_json::json!({
                            "score": round_score(chunk.score),
                            "content": chunk.chunk.chunk
                        })
                    }).collect::<Vec<_>>()
                }
            })
        })
        .collect::<Vec<_>>())
}

fn round_score(value: f32) -> f64 {
    (f64::from(value) * 1000.0).round() / 1000.0
}
