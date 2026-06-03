use async_trait::async_trait;
use common::{
    error::AppError,
    storage::types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk},
};
use fastembed::RerankResult;
use std::collections::HashMap;
use tracing::{debug, instrument, warn};

use crate::{
    query::normalize_fts_terms,
    scoring::{clamp_unit, min_max_normalize, reciprocal_rank_fusion, RrfConfig, Scored},
    RetrievedChunk, RetrievedEntity,
};

use super::{
    config::RetrievalTuning,
    context::PipelineContext,
    diagnostics::{AssembleStats, SearchStats},
    Stage, StageKind,
};

#[derive(Debug, Clone, Copy)]
pub struct EmbedStage;

#[async_trait]
impl Stage for EmbedStage {
    fn kind(&self) -> StageKind {
        StageKind::Embed
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        embed(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkSearchStage;

#[async_trait]
impl Stage for ChunkSearchStage {
    fn kind(&self) -> StageKind {
        StageKind::Search
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        search_chunks(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkRerankStage;

#[async_trait]
impl Stage for ChunkRerankStage {
    fn kind(&self) -> StageKind {
        StageKind::Rerank
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        rerank_chunks(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolveEntitiesStage;

#[async_trait]
impl Stage for ResolveEntitiesStage {
    fn kind(&self) -> StageKind {
        StageKind::ResolveEntities
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        resolve_entities(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkAssembleStage;

#[async_trait]
impl Stage for ChunkAssembleStage {
    fn kind(&self) -> StageKind {
        StageKind::Assemble
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        assemble_chunks(ctx)
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn embed(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    if ctx.query_embedding.is_some() {
        debug!("Reusing cached query embedding for hybrid retrieval");
    } else {
        debug!("Generating query embedding for hybrid retrieval");
        let embedding = ctx.embedding_provider.embed(&ctx.input_text).await?;
        ctx.query_embedding = Some(embedding);
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn search_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Collecting chunk candidates via vector and FTS search");
    let embedding = ctx.ensure_embedding().map_err(|e| *e)?.clone();
    let tuning = &ctx.config.tuning;
    let fts_take = tuning.chunk_fts_take;
    let (fts_query, fts_token_count) = normalize_fts_terms(&ctx.input_text);
    let fts_enabled = tuning.flags.chunk_rrf_use_fts() && fts_take > 0 && !fts_query.is_empty();

    let (vector_rows, fts_rows) = tokio::try_join!(
        TextChunk::vector_search(
            tuning.chunk_vector_take,
            embedding,
            ctx.db_client,
            &ctx.user_id,
        ),
        async {
            if fts_enabled {
                TextChunk::fts_search(fts_take, &fts_query, ctx.db_client, &ctx.user_id).await
            } else {
                Ok(Vec::new())
            }
        }
    )?;

    let vector_candidates = vector_rows.len();
    let fts_candidates = fts_rows.len();

    let vector_scored: Vec<Scored<TextChunk>> = vector_rows
        .into_iter()
        .map(|row| Scored::new(row.chunk).with_vector_score(row.score))
        .collect();

    let fts_scored: Vec<Scored<TextChunk>> = fts_rows
        .into_iter()
        .map(|row| Scored::new(row.chunk).with_fts_score(row.score))
        .collect();

    let mut fts_weight = tuning.chunk_rrf_fts_weight;
    if fts_enabled && fts_token_count > 0 && fts_token_count <= 3 {
        // For very short keyword queries, lean more on lexical ranking.
        fts_weight *= 1.5;
    }

    let rrf_config = RrfConfig {
        k: tuning.chunk_rrf_k,
        vector_weight: tuning.chunk_rrf_vector_weight,
        fts_weight,
        use_vector: tuning.flags.chunk_rrf_use_vector(),
        use_fts: tuning.flags.chunk_rrf_use_fts() && fts_candidates > 0,
    };

    let chunks = reciprocal_rank_fusion(vector_scored, fts_scored, rrf_config);

    debug!(
        total_merged = chunks.len(),
        vector_only = chunks.iter().filter(|c| c.scores.fts.is_none()).count(),
        fts_only = chunks.iter().filter(|c| c.scores.vector.is_none()).count(),
        both_signals = chunks
            .iter()
            .filter(|c| c.scores.vector.is_some() && c.scores.fts.is_some())
            .count(),
        rrf_k = rrf_config.k,
        "Merged chunk candidates with RRF"
    );

    if ctx.diagnostics_enabled() {
        ctx.record_search(SearchStats {
            vector_chunk_candidates: vector_candidates,
            fts_chunk_candidates: fts_candidates,
            vector_chunk_scores: sample_scores(&chunks, |chunk| chunk.scores.vector.unwrap_or(0.0)),
            fts_chunk_scores: sample_scores(&chunks, |chunk| chunk.scores.fts.unwrap_or(0.0)),
        });
    }

    ctx.chunk_values = chunks;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn rerank_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    if ctx.chunk_values.len() <= 1 {
        return Ok(());
    }

    let Some(reranker) = ctx.reranker.as_ref() else {
        debug!("No reranker lease provided; skipping chunk rerank stage");
        return Ok(());
    };

    let documents =
        build_chunk_rerank_documents(&ctx.chunk_values, ctx.config.tuning.rerank_keep_top.max(1));
    if documents.len() <= 1 {
        debug!("Skipping chunk reranking stage; insufficient chunk documents");
        return Ok(());
    }

    match reranker.rerank(&ctx.input_text, documents).await {
        Ok(results) if !results.is_empty() => {
            apply_chunk_rerank_results(&mut ctx.chunk_values, &ctx.config.tuning, results);
        }
        Ok(_) => debug!("Chunk reranker returned no results; retaining original order"),
        Err(err) => warn!(
            error = %err,
            "Chunk reranking failed; continuing with original ordering"
        ),
    }

    Ok(())
}

/// Resolve the `KnowledgeEntity` rows that own the retrieved chunks.
///
/// Entities are derived directly from the (benchmarked) chunk retrieval: chunks are grouped
/// by `source_id`, the owning entities are loaded, scored by their best contributing chunk,
/// and the contributing chunks are attached.
#[instrument(level = "trace", skip_all)]
pub async fn resolve_entities(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    if ctx.chunk_values.is_empty() {
        return Ok(());
    }

    let max_chunks = ctx.config.tuning.max_chunks_per_entity.max(1);

    let mut source_order: Vec<String> = Vec::new();
    let mut chunks_by_source: HashMap<String, Vec<RetrievedChunk>> = HashMap::new();
    let mut best_score: HashMap<String, f32> = HashMap::new();

    for scored in &ctx.chunk_values {
        let source = scored.item.source_id.clone();
        let attached = chunks_by_source.entry(source.clone()).or_default();
        if attached.is_empty() {
            source_order.push(source.clone());
            best_score.insert(source.clone(), scored.fused);
        }
        if attached.len() < max_chunks {
            attached.push(RetrievedChunk {
                chunk: scored.item.clone(),
                score: scored.fused,
            });
        }
    }

    let entities =
        KnowledgeEntity::find_by_source_ids(ctx.db_client, &source_order, &ctx.user_id).await?;

    let mut entities_by_source: HashMap<String, Vec<KnowledgeEntity>> = HashMap::new();
    for entity in entities {
        entities_by_source
            .entry(entity.source_id.clone())
            .or_default()
            .push(entity);
    }

    let mut results = Vec::new();
    for source in &source_order {
        let Some(entities) = entities_by_source.remove(source) else {
            continue;
        };
        let score = best_score.get(source).copied().unwrap_or(0.0);
        let chunks = chunks_by_source.get(source).cloned().unwrap_or_default();
        for entity in entities {
            results.push(RetrievedEntity {
                entity,
                score,
                chunks: chunks.clone(),
            });
        }
    }

    debug!(
        sources = source_order.len(),
        entities = results.len(),
        "Resolved entities from retrieved chunks"
    );

    ctx.entity_results = results;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
#[allow(clippy::result_large_err)]
pub fn assemble_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Assembling chunk retrieval results");
    let mut chunk_values = std::mem::take(&mut ctx.chunk_values);
    // Limit how many chunks we return to keep context size reasonable.
    let limit = ctx
        .config
        .tuning
        .chunk_result_cap
        .max(1)
        .min(ctx.config.tuning.chunk_vector_take.max(1));

    if chunk_values.len() > limit {
        chunk_values.truncate(limit);
    }

    ctx.chunk_results = chunk_values
        .into_iter()
        .map(|chunk| RetrievedChunk {
            chunk: chunk.item,
            score: chunk.fused,
        })
        .collect();

    if ctx.diagnostics_enabled() {
        ctx.record_assemble(AssembleStats {
            chunks_selected: ctx.chunk_results.len(),
        });
    }

    Ok(())
}

const SCORE_SAMPLE_LIMIT: usize = 8;

fn sample_scores<T, F>(items: &[Scored<T>], extractor: F) -> Vec<f32>
where
    F: FnMut(&Scored<T>) -> f32,
{
    items.iter().take(SCORE_SAMPLE_LIMIT).map(extractor).collect()
}

fn build_chunk_rerank_documents(chunks: &[Scored<TextChunk>], max_chunks: usize) -> Vec<String> {
    chunks
        .iter()
        .take(max_chunks)
        .map(|chunk| {
            format!(
                "Source: {}\nChunk:\n{}",
                chunk.item.source_id,
                chunk.item.chunk.trim()
            )
        })
        .collect()
}

fn apply_chunk_rerank_results(
    chunks: &mut Vec<Scored<TextChunk>>,
    tuning: &RetrievalTuning,
    results: Vec<RerankResult>,
) {
    if results.is_empty() || chunks.is_empty() {
        return;
    }

    let mut remaining: Vec<Option<Scored<TextChunk>>> =
        std::mem::take(chunks).into_iter().map(Some).collect();

    let raw_scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let normalized_scores = min_max_normalize(&raw_scores);

    let use_only = tuning.flags.rerank_scores_only();
    let blend = if use_only {
        1.0
    } else {
        clamp_unit(tuning.rerank_blend_weight)
    };

    let mut reranked: Vec<Scored<TextChunk>> = Vec::with_capacity(remaining.len());
    for (result, normalized) in results.into_iter().zip(normalized_scores.into_iter()) {
        if let Some(slot) = remaining.get_mut(result.index) {
            if let Some(mut candidate) = slot.take() {
                let original = candidate.fused;
                let blended = if use_only {
                    clamp_unit(normalized)
                } else {
                    clamp_unit(original * (1.0 - blend) + normalized * blend)
                };
                candidate.update_fused(blended);
                reranked.push(candidate);
            }
        } else {
            warn!(
                result_index = result.index,
                "Chunk reranker returned out-of-range index; skipping"
            );
        }
        if reranked.len() == remaining.len() {
            break;
        }
    }

    reranked.extend(remaining.into_iter().flatten());

    let keep_top = tuning.rerank_keep_top;
    if keep_top > 0 && reranked.len() > keep_top {
        reranked.truncate(keep_top);
    }

    *chunks = reranked;
}
