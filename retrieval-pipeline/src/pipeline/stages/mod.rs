use async_openai::Client;
use async_trait::async_trait;
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk, StoredObject},
    },
    utils::{embedding::generate_embedding, embedding::EmbeddingProvider},
};
use fastembed::RerankResult;
use futures::{stream::FuturesUnordered, StreamExt};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};
use tracing::{debug, instrument, warn};

use crate::{
    fts::find_items_by_fts,
    graph::{find_entities_by_relationship_by_id, find_entities_by_source_ids},
    reranking::RerankerLease,
    scoring::{
        clamp_unit, fuse_scores, merge_scored_by_id, min_max_normalize, sort_by_fused_desc,
        FusionWeights, Scored,
    },
    RetrievedChunk, RetrievedEntity,
};

use super::{
    config::{RetrievalConfig, RetrievalTuning},
    diagnostics::{
        AssembleStats, ChunkEnrichmentStats, CollectCandidatesStats, EntityAssemblyTrace,
        PipelineDiagnostics,
    },
    PipelineStage, PipelineStageTimings, StageKind,
};

pub struct PipelineContext<'a> {
    pub db_client: &'a SurrealDbClient,
    pub openai_client: &'a Client<async_openai::config::OpenAIConfig>,
    pub embedding_provider: Option<&'a EmbeddingProvider>,
    pub input_text: String,
    pub user_id: String,
    pub config: RetrievalConfig,
    pub query_embedding: Option<Vec<f32>>,
    pub entity_candidates: HashMap<String, Scored<KnowledgeEntity>>,
    pub chunk_candidates: HashMap<String, Scored<TextChunk>>,
    pub filtered_entities: Vec<Scored<KnowledgeEntity>>,
    pub chunk_values: Vec<Scored<TextChunk>>,
    pub revised_chunk_values: Vec<Scored<TextChunk>>,
    pub reranker: Option<RerankerLease>,
    pub diagnostics: Option<PipelineDiagnostics>,
    pub entity_results: Vec<RetrievedEntity>,
    pub chunk_results: Vec<RetrievedChunk>,
    stage_timings: PipelineStageTimings,
}

impl<'a> PipelineContext<'a> {
    pub fn new(
        db_client: &'a SurrealDbClient,
        openai_client: &'a Client<async_openai::config::OpenAIConfig>,
        embedding_provider: Option<&'a EmbeddingProvider>,
        input_text: String,
        user_id: String,
        config: RetrievalConfig,
        reranker: Option<RerankerLease>,
    ) -> Self {
        Self {
            db_client,
            openai_client,
            embedding_provider,
            input_text,
            user_id,
            config,
            query_embedding: None,
            entity_candidates: HashMap::new(),
            chunk_candidates: HashMap::new(),
            filtered_entities: Vec::new(),
            chunk_values: Vec::new(),
            revised_chunk_values: Vec::new(),
            reranker,
            diagnostics: None,
            entity_results: Vec::new(),
            chunk_results: Vec::new(),
            stage_timings: PipelineStageTimings::default(),
        }
    }

    pub fn with_embedding(
        db_client: &'a SurrealDbClient,
        openai_client: &'a Client<async_openai::config::OpenAIConfig>,
        embedding_provider: Option<&'a EmbeddingProvider>,
        query_embedding: Vec<f32>,
        input_text: String,
        user_id: String,
        config: RetrievalConfig,
        reranker: Option<RerankerLease>,
    ) -> Self {
        let mut ctx = Self::new(
            db_client,
            openai_client,
            embedding_provider,
            input_text,
            user_id,
            config,
            reranker,
        );
        ctx.query_embedding = Some(query_embedding);
        ctx
    }

    fn ensure_embedding(&self) -> Result<&Vec<f32>, AppError> {
        self.query_embedding.as_ref().ok_or_else(|| {
            AppError::InternalError(
                "query embedding missing before candidate collection".to_string(),
            )
        })
    }

    pub fn enable_diagnostics(&mut self) {
        if self.diagnostics.is_none() {
            self.diagnostics = Some(PipelineDiagnostics::default());
        }
    }

    pub fn diagnostics_enabled(&self) -> bool {
        self.diagnostics.is_some()
    }

    pub fn record_collect_candidates(&mut self, stats: CollectCandidatesStats) {
        if let Some(diag) = self.diagnostics.as_mut() {
            diag.collect_candidates = Some(stats);
        }
    }

    pub fn record_chunk_enrichment(&mut self, stats: ChunkEnrichmentStats) {
        if let Some(diag) = self.diagnostics.as_mut() {
            diag.enrich_chunks_from_entities = Some(stats);
        }
    }

    pub fn record_assemble(&mut self, stats: AssembleStats) {
        if let Some(diag) = self.diagnostics.as_mut() {
            diag.assemble = Some(stats);
        }
    }

    pub fn take_diagnostics(&mut self) -> Option<PipelineDiagnostics> {
        self.diagnostics.take()
    }

    pub fn take_stage_timings(&mut self) -> PipelineStageTimings {
        std::mem::take(&mut self.stage_timings)
    }

    pub fn record_stage_duration(&mut self, kind: StageKind, duration: std::time::Duration) {
        self.stage_timings.record(kind, duration);
    }

    pub fn take_entity_results(&mut self) -> Vec<RetrievedEntity> {
        std::mem::take(&mut self.entity_results)
    }

    pub fn take_chunk_results(&mut self) -> Vec<RetrievedChunk> {
        std::mem::take(&mut self.chunk_results)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EmbedStage;

#[async_trait]
impl PipelineStage for EmbedStage {
    fn kind(&self) -> StageKind {
        StageKind::Embed
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        embed(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CollectCandidatesStage;

#[async_trait]
impl PipelineStage for CollectCandidatesStage {
    fn kind(&self) -> StageKind {
        StageKind::CollectCandidates
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        collect_candidates(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GraphExpansionStage;

#[async_trait]
impl PipelineStage for GraphExpansionStage {
    fn kind(&self) -> StageKind {
        StageKind::GraphExpansion
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        expand_graph(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkAttachStage;

#[async_trait]
impl PipelineStage for ChunkAttachStage {
    fn kind(&self) -> StageKind {
        StageKind::ChunkAttach
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        attach_chunks(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RerankStage;

#[async_trait]
impl PipelineStage for RerankStage {
    fn kind(&self) -> StageKind {
        StageKind::Rerank
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        rerank(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AssembleEntitiesStage;

#[async_trait]
impl PipelineStage for AssembleEntitiesStage {
    fn kind(&self) -> StageKind {
        StageKind::Assemble
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        assemble(ctx)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkVectorStage;

#[async_trait]
impl PipelineStage for ChunkVectorStage {
    fn kind(&self) -> StageKind {
        StageKind::CollectCandidates
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        collect_vector_chunks(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkRerankStage;

#[async_trait]
impl PipelineStage for ChunkRerankStage {
    fn kind(&self) -> StageKind {
        StageKind::Rerank
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        rerank_chunks(ctx).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkAssembleStage;

#[async_trait]
impl PipelineStage for ChunkAssembleStage {
    fn kind(&self) -> StageKind {
        StageKind::Assemble
    }

    async fn execute(&self, ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
        assemble_chunks(ctx)
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn embed(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    let embedding_cached = ctx.query_embedding.is_some();
    if embedding_cached {
        debug!("Reusing cached query embedding for hybrid retrieval");
    } else {
        debug!("Generating query embedding for hybrid retrieval");
        let embedding = if let Some(provider) = ctx.embedding_provider {
            provider.embed(&ctx.input_text).await.map_err(|e| {
                AppError::InternalError(format!(
                    "Failed to generate embedding with provider: {}",
                    e
                ))
            })?
        } else {
            generate_embedding(ctx.openai_client, &ctx.input_text, ctx.db_client).await?
        };
        ctx.query_embedding = Some(embedding);
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn collect_candidates(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Collecting initial candidates via vector and FTS search");
    let embedding = ctx.ensure_embedding()?.clone();
    let tuning = &ctx.config.tuning;

    let weights = FusionWeights::default();

    let (vector_entity_results, vector_chunk_results, mut fts_entities, mut fts_chunks) = tokio::try_join!(
        KnowledgeEntity::vector_search(
            tuning.entity_vector_take,
            embedding.clone(),
            ctx.db_client,
            &ctx.user_id,
        ),
        TextChunk::vector_search(
            tuning.chunk_vector_take,
            embedding,
            ctx.db_client,
            &ctx.user_id,
        ),
        find_items_by_fts(
            tuning.entity_fts_take,
            &ctx.input_text,
            ctx.db_client,
            "knowledge_entity",
            &ctx.user_id,
        ),
        find_items_by_fts(
            tuning.chunk_fts_take,
            &ctx.input_text,
            ctx.db_client,
            "text_chunk",
            &ctx.user_id
        ),
    )?;

    let vector_entities: Vec<Scored<KnowledgeEntity>> = vector_entity_results
        .into_iter()
        .map(|row| Scored::new(row.entity).with_vector_score(row.score))
        .collect();
    let vector_chunks: Vec<Scored<TextChunk>> = vector_chunk_results
        .into_iter()
        .map(|row| Scored::new(row.chunk).with_vector_score(row.score))
        .collect();

    debug!(
        vector_entities = vector_entities.len(),
        vector_chunks = vector_chunks.len(),
        fts_entities = fts_entities.len(),
        fts_chunks = fts_chunks.len(),
        "Hybrid retrieval initial candidate counts"
    );

    if ctx.diagnostics_enabled() {
        ctx.record_collect_candidates(CollectCandidatesStats {
            vector_entity_candidates: vector_entities.len(),
            vector_chunk_candidates: vector_chunks.len(),
            fts_entity_candidates: fts_entities.len(),
            fts_chunk_candidates: fts_chunks.len(),
            vector_chunk_scores: sample_scores(&vector_chunks, |chunk| {
                chunk.scores.vector.unwrap_or(0.0)
            }),
            fts_chunk_scores: sample_scores(&fts_chunks, |chunk| chunk.scores.fts.unwrap_or(0.0)),
        });
    }

    normalize_fts_scores(&mut fts_entities);
    normalize_fts_scores(&mut fts_chunks);

    merge_scored_by_id(&mut ctx.entity_candidates, vector_entities);
    merge_scored_by_id(&mut ctx.entity_candidates, fts_entities);
    merge_scored_by_id(&mut ctx.chunk_candidates, vector_chunks);
    merge_scored_by_id(&mut ctx.chunk_candidates, fts_chunks);

    apply_fusion(&mut ctx.entity_candidates, weights);
    apply_fusion(&mut ctx.chunk_candidates, weights);

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn expand_graph(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Expanding candidates using graph relationships");
    let tuning = &ctx.config.tuning;
    let weights = FusionWeights::default();

    if ctx.entity_candidates.is_empty() {
        return Ok(());
    }

    let graph_seeds = seeds_from_candidates(
        &ctx.entity_candidates,
        tuning.graph_seed_min_score,
        tuning.graph_traversal_seed_limit,
    );

    if graph_seeds.is_empty() {
        return Ok(());
    }

    let mut futures = FuturesUnordered::new();
    for seed in graph_seeds {
        let db = ctx.db_client;
        let user = ctx.user_id.clone();
        let limit = tuning.graph_neighbor_limit;
        futures.push(async move {
            let neighbors = find_entities_by_relationship_by_id(db, &seed.id, &user, limit).await;
            (seed, neighbors)
        });
    }

    while let Some((seed, neighbors_result)) = futures.next().await {
        let neighbors = neighbors_result.map_err(AppError::from)?;
        if neighbors.is_empty() {
            continue;
        }

        for neighbor in neighbors {
            let neighbor_id = neighbor.id.clone();
            if neighbor_id == seed.id {
                continue;
            }

            let graph_score = clamp_unit(seed.fused * tuning.graph_score_decay);
            let entry = ctx
                .entity_candidates
                .entry(neighbor_id.clone())
                .or_insert_with(|| Scored::new(neighbor.clone()));

            entry.item = neighbor;

            let inherited_vector = clamp_unit(graph_score * tuning.graph_vector_inheritance);
            let vector_existing = entry.scores.vector.unwrap_or(0.0);
            if inherited_vector > vector_existing {
                entry.scores.vector = Some(inherited_vector);
            }

            let existing_graph = entry.scores.graph.unwrap_or(f32::MIN);
            if graph_score > existing_graph || entry.scores.graph.is_none() {
                entry.scores.graph = Some(graph_score);
            }

            let fused = fuse_scores(&entry.scores, weights);
            entry.update_fused(fused);
        }
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn attach_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Attaching chunks to surviving entities");
    let tuning = &ctx.config.tuning;
    let weights = FusionWeights::default();

    let chunk_by_source = group_chunks_by_source(&ctx.chunk_candidates);
    let chunk_candidates_before = ctx.chunk_candidates.len();
    let chunk_sources_considered = chunk_by_source.len();

    backfill_entities_from_chunks(
        &mut ctx.entity_candidates,
        &chunk_by_source,
        ctx.db_client,
        &ctx.user_id,
        weights,
    )
    .await?;

    boost_entities_with_chunks(&mut ctx.entity_candidates, &chunk_by_source, weights);

    let mut entity_results: Vec<Scored<KnowledgeEntity>> =
        ctx.entity_candidates.values().cloned().collect();
    sort_by_fused_desc(&mut entity_results);

    let mut filtered_entities: Vec<Scored<KnowledgeEntity>> = entity_results
        .iter()
        .filter(|candidate| candidate.fused >= tuning.score_threshold)
        .cloned()
        .collect();

    if filtered_entities.len() < tuning.fallback_min_results {
        filtered_entities = entity_results
            .into_iter()
            .take(tuning.fallback_min_results)
            .collect();
    }

    ctx.filtered_entities = filtered_entities;

    let mut chunk_results: Vec<Scored<TextChunk>> =
        ctx.chunk_candidates.values().cloned().collect();
    sort_by_fused_desc(&mut chunk_results);

    let mut chunk_by_id: HashMap<String, Scored<TextChunk>> = HashMap::new();
    for chunk in chunk_results {
        chunk_by_id.insert(chunk.item.id.clone(), chunk);
    }

    enrich_chunks_from_entities(
        &mut chunk_by_id,
        &ctx.filtered_entities,
        ctx.db_client,
        &ctx.user_id,
        weights,
    )
    .await?;

    let mut chunk_values: Vec<Scored<TextChunk>> = chunk_by_id.into_values().collect();
    sort_by_fused_desc(&mut chunk_values);

    if ctx.diagnostics_enabled() {
        ctx.record_chunk_enrichment(ChunkEnrichmentStats {
            filtered_entity_count: ctx.filtered_entities.len(),
            fallback_min_results: tuning.fallback_min_results,
            chunk_sources_considered,
            chunk_candidates_before_enrichment: chunk_candidates_before,
            chunk_candidates_after_enrichment: chunk_values.len(),
            top_chunk_scores: sample_scores(&chunk_values, |chunk| chunk.fused),
        });
    }

    ctx.chunk_values = chunk_values;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn rerank(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    let mut applied = false;

    if let Some(reranker) = ctx.reranker.as_ref() {
        if ctx.filtered_entities.len() > 1 {
            let documents = build_rerank_documents(ctx, ctx.config.tuning.max_chunks_per_entity);

            if documents.len() > 1 {
                match reranker.rerank(&ctx.input_text, documents).await {
                    Ok(results) if !results.is_empty() => {
                        apply_rerank_results(ctx, results);
                        applied = true;
                    }
                    Ok(_) => {
                        debug!("Reranker returned no results; retaining original ordering");
                    }
                    Err(err) => {
                        warn!(
                            error = %err,
                            "Reranking failed; continuing with original ordering"
                        );
                    }
                }
            } else {
                debug!(
                    document_count = documents.len(),
                    "Skipping reranking stage; insufficient document context"
                );
            }
        } else {
            debug!("Skipping reranking stage; less than two entities available");
        }
    } else {
        debug!("No reranker lease provided; skipping reranking stage");
    }

    if applied {
        debug!("Applied reranking adjustments to candidate ordering");
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn collect_vector_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Collecting vector chunk candidates for revised strategy");
    let embedding = ctx.ensure_embedding()?.clone();
    let tuning = &ctx.config.tuning;
    let weights = FusionWeights::default();

    let mut vector_chunks: Vec<Scored<TextChunk>> = TextChunk::vector_search(
        tuning.chunk_vector_take,
        embedding,
        ctx.db_client,
        &ctx.user_id,
    )
    .await?
    .into_iter()
    .map(|row| {
        let mut scored = Scored::new(row.chunk).with_vector_score(row.score);
        let fused = fuse_scores(&scored.scores, weights);
        scored.update_fused(fused);
        scored
    })
    .collect();

    if ctx.diagnostics_enabled() {
        ctx.record_collect_candidates(CollectCandidatesStats {
            vector_entity_candidates: 0,
            vector_chunk_candidates: vector_chunks.len(),
            fts_entity_candidates: 0,
            fts_chunk_candidates: 0,
            vector_chunk_scores: sample_scores(&vector_chunks, |chunk| {
                chunk.scores.vector.unwrap_or(0.0)
            }),
            fts_chunk_scores: Vec::new(),
        });
    }

    vector_chunks.sort_by(|a, b| b.fused.partial_cmp(&a.fused).unwrap_or(Ordering::Equal));
    ctx.revised_chunk_values = vector_chunks;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn rerank_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    if ctx.revised_chunk_values.len() <= 1 {
        return Ok(());
    }

    let Some(reranker) = ctx.reranker.as_ref() else {
        debug!("No reranker lease provided; skipping chunk rerank stage");
        return Ok(());
    };

    let documents = build_chunk_rerank_documents(
        &ctx.revised_chunk_values,
        ctx.config.tuning.rerank_keep_top.max(1),
    );
    if documents.len() <= 1 {
        debug!("Skipping chunk reranking stage; insufficient chunk documents");
        return Ok(());
    }

    match reranker.rerank(&ctx.input_text, documents).await {
        Ok(results) if !results.is_empty() => {
            apply_chunk_rerank_results(&mut ctx.revised_chunk_values, &ctx.config.tuning, results);
        }
        Ok(_) => debug!("Chunk reranker returned no results; retaining original order"),
        Err(err) => warn!(
            error = %err,
            "Chunk reranking failed; continuing with original ordering"
        ),
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn assemble_chunks(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Assembling chunk-only retrieval results");
    let mut chunk_values = std::mem::take(&mut ctx.revised_chunk_values);
    let question_terms = extract_keywords(&ctx.input_text);
    rank_chunks_by_combined_score(
        &mut chunk_values,
        &question_terms,
        ctx.config.tuning.lexical_match_weight,
    );

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
            token_budget_start: ctx.config.tuning.token_budget_estimate,
            token_budget_spent: 0,
            token_budget_remaining: ctx.config.tuning.token_budget_estimate,
            budget_exhausted: false,
            chunks_selected: ctx.chunk_results.len(),
            chunks_skipped_due_budget: 0,
            entity_count: 0,
            entity_traces: Vec::new(),
        });
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn assemble(ctx: &mut PipelineContext<'_>) -> Result<(), AppError> {
    debug!("Assembling final retrieved entities");
    let tuning = &ctx.config.tuning;
    let question_terms = extract_keywords(&ctx.input_text);

    let mut chunk_by_source: HashMap<String, Vec<Scored<TextChunk>>> = HashMap::new();
    for chunk in ctx.chunk_values.drain(..) {
        chunk_by_source
            .entry(chunk.item.source_id.clone())
            .or_default()
            .push(chunk);
    }

    for chunk_list in chunk_by_source.values_mut() {
        chunk_list.sort_by(|a, b| {
            // No base-table embeddings; order by fused score only.
            b.fused.partial_cmp(&a.fused).unwrap_or(Ordering::Equal)
        });
    }

    let mut token_budget_remaining = tuning.token_budget_estimate;
    let mut results = Vec::new();
    let diagnostics_enabled = ctx.diagnostics_enabled();
    let mut per_entity_traces = Vec::new();
    let mut chunks_skipped_due_budget = 0usize;
    let mut chunks_selected = 0usize;
    let mut tokens_spent = 0usize;

    for entity in &ctx.filtered_entities {
        let mut selected_chunks = Vec::new();
        let mut entity_trace = if diagnostics_enabled {
            Some(EntityAssemblyTrace {
                entity_id: entity.item.id.clone(),
                source_id: entity.item.source_id.clone(),
                inspected_candidates: 0,
                selected_chunk_ids: Vec::new(),
                selected_chunk_scores: Vec::new(),
                skipped_due_budget: 0,
            })
        } else {
            None
        };
        if let Some(candidates) = chunk_by_source.get_mut(&entity.item.source_id) {
            rank_chunks_by_combined_score(candidates, &question_terms, tuning.lexical_match_weight);
            let mut per_entity_count = 0;
            for candidate in candidates.iter() {
                if let Some(trace) = entity_trace.as_mut() {
                    trace.inspected_candidates += 1;
                }
                if per_entity_count >= tuning.max_chunks_per_entity {
                    break;
                }
                let estimated_tokens =
                    estimate_tokens(&candidate.item.chunk, tuning.avg_chars_per_token);
                if estimated_tokens > token_budget_remaining {
                    chunks_skipped_due_budget += 1;
                    if let Some(trace) = entity_trace.as_mut() {
                        trace.skipped_due_budget += 1;
                    }
                    continue;
                }

                token_budget_remaining = token_budget_remaining.saturating_sub(estimated_tokens);
                tokens_spent += estimated_tokens;
                per_entity_count += 1;
                chunks_selected += 1;

                selected_chunks.push(RetrievedChunk {
                    chunk: candidate.item.clone(),
                    score: candidate.fused,
                });
                if let Some(trace) = entity_trace.as_mut() {
                    trace.selected_chunk_ids.push(candidate.item.id.clone());
                    trace.selected_chunk_scores.push(candidate.fused);
                }
            }
        }

        results.push(RetrievedEntity {
            entity: entity.item.clone(),
            score: entity.fused,
            chunks: selected_chunks,
        });

        if let Some(trace) = entity_trace {
            per_entity_traces.push(trace);
        }

        if token_budget_remaining == 0 {
            break;
        }
    }

    if diagnostics_enabled {
        ctx.record_assemble(AssembleStats {
            token_budget_start: tuning.token_budget_estimate,
            token_budget_spent: tokens_spent,
            token_budget_remaining,
            budget_exhausted: token_budget_remaining == 0,
            chunks_selected,
            chunks_skipped_due_budget,
            entity_count: ctx.filtered_entities.len(),
            entity_traces: per_entity_traces,
        });
    }

    ctx.entity_results = results;
    Ok(())
}

const SCORE_SAMPLE_LIMIT: usize = 8;

fn sample_scores<T, F>(items: &[Scored<T>], mut extractor: F) -> Vec<f32>
where
    F: FnMut(&Scored<T>) -> f32,
{
    items
        .iter()
        .take(SCORE_SAMPLE_LIMIT)
        .map(|item| extractor(item))
        .collect()
}

fn normalize_fts_scores<T>(results: &mut [Scored<T>]) {
    let raw_scores: Vec<f32> = results
        .iter()
        .map(|candidate| candidate.scores.fts.unwrap_or(0.0))
        .collect();

    let normalized = min_max_normalize(&raw_scores);
    for (candidate, normalized_score) in results.iter_mut().zip(normalized.into_iter()) {
        candidate.scores.fts = Some(normalized_score);
        candidate.update_fused(0.0);
    }
}

fn apply_fusion<T>(candidates: &mut HashMap<String, Scored<T>>, weights: FusionWeights)
where
    T: StoredObject,
{
    for candidate in candidates.values_mut() {
        let fused = fuse_scores(&candidate.scores, weights);
        candidate.update_fused(fused);
    }
}

fn group_chunks_by_source(
    chunks: &HashMap<String, Scored<TextChunk>>,
) -> HashMap<String, Vec<Scored<TextChunk>>> {
    let mut by_source: HashMap<String, Vec<Scored<TextChunk>>> = HashMap::new();

    for chunk in chunks.values() {
        by_source
            .entry(chunk.item.source_id.clone())
            .or_default()
            .push(chunk.clone());
    }
    by_source
}

async fn backfill_entities_from_chunks(
    entity_candidates: &mut HashMap<String, Scored<KnowledgeEntity>>,
    chunk_by_source: &HashMap<String, Vec<Scored<TextChunk>>>,
    db_client: &SurrealDbClient,
    user_id: &str,
    weights: FusionWeights,
) -> Result<(), AppError> {
    let mut missing_sources = Vec::new();

    for source_id in chunk_by_source.keys() {
        if !entity_candidates
            .values()
            .any(|entity| entity.item.source_id == *source_id)
        {
            missing_sources.push(source_id.clone());
        }
    }

    if missing_sources.is_empty() {
        return Ok(());
    }

    let related_entities: Vec<KnowledgeEntity> = find_entities_by_source_ids(
        missing_sources.clone(),
        "knowledge_entity",
        user_id,
        db_client,
    )
    .await
    .unwrap_or_default();

    if related_entities.is_empty() {
        warn!("expected related entities for missing chunk sources, but none were found");
    }

    for entity in related_entities {
        if let Some(chunks) = chunk_by_source.get(&entity.source_id) {
            let best_chunk_score = chunks
                .iter()
                .map(|chunk| chunk.fused)
                .fold(0.0f32, f32::max);

            let mut scored = Scored::new(entity.clone()).with_vector_score(best_chunk_score);
            let fused = fuse_scores(&scored.scores, weights);
            scored.update_fused(fused);
            entity_candidates.insert(entity.id.clone(), scored);
        }
    }

    Ok(())
}

fn boost_entities_with_chunks(
    entity_candidates: &mut HashMap<String, Scored<KnowledgeEntity>>,
    chunk_by_source: &HashMap<String, Vec<Scored<TextChunk>>>,
    weights: FusionWeights,
) {
    for entity in entity_candidates.values_mut() {
        if let Some(chunks) = chunk_by_source.get(&entity.item.source_id) {
            let best_chunk_score = chunks
                .iter()
                .map(|chunk| chunk.fused)
                .fold(0.0f32, f32::max);

            if best_chunk_score > 0.0 {
                let boosted = entity.scores.vector.unwrap_or(0.0).max(best_chunk_score);
                entity.scores.vector = Some(boosted);
                let fused = fuse_scores(&entity.scores, weights);
                entity.update_fused(fused);
            }
        }
    }
}

async fn enrich_chunks_from_entities(
    chunk_candidates: &mut HashMap<String, Scored<TextChunk>>,
    entities: &[Scored<KnowledgeEntity>],
    db_client: &SurrealDbClient,
    user_id: &str,
    weights: FusionWeights,
) -> Result<(), AppError> {
    let mut source_ids: HashSet<String> = HashSet::new();
    for entity in entities {
        source_ids.insert(entity.item.source_id.clone());
    }

    if source_ids.is_empty() {
        return Ok(());
    }

    let chunks = find_entities_by_source_ids::<TextChunk>(
        source_ids.into_iter().collect(),
        "text_chunk",
        user_id,
        db_client,
    )
    .await?;

    let mut entity_score_lookup: HashMap<String, f32> = HashMap::new();
    for entity in entities {
        entity_score_lookup.insert(entity.item.source_id.clone(), entity.fused);
    }

    for chunk in chunks {
        let entry = chunk_candidates
            .entry(chunk.id.clone())
            .or_insert_with(|| Scored::new(chunk.clone()).with_vector_score(0.0));

        let entity_score = entity_score_lookup
            .get(&chunk.source_id)
            .copied()
            .unwrap_or(0.0);

        entry.scores.vector = Some(entry.scores.vector.unwrap_or(0.0).max(entity_score * 0.8));
        let fused = fuse_scores(&entry.scores, weights);
        entry.update_fused(fused);
        entry.item = chunk;
    }

    Ok(())
}

fn build_rerank_documents(ctx: &PipelineContext<'_>, max_chunks_per_entity: usize) -> Vec<String> {
    if ctx.filtered_entities.is_empty() {
        return Vec::new();
    }

    let mut chunk_by_source: HashMap<&str, Vec<&Scored<TextChunk>>> = HashMap::new();
    for chunk in &ctx.chunk_values {
        chunk_by_source
            .entry(chunk.item.source_id.as_str())
            .or_default()
            .push(chunk);
    }

    ctx.filtered_entities
        .iter()
        .map(|entity| {
            let mut doc = format!(
                "Name: {}\nType: {:?}\nDescription: {}\n",
                entity.item.name, entity.item.entity_type, entity.item.description
            );

            if let Some(chunks) = chunk_by_source.get(entity.item.source_id.as_str()) {
                let mut chunk_refs = chunks.clone();
                chunk_refs.sort_by(|a, b| {
                    b.fused
                        .partial_cmp(&a.fused)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                let mut header_added = false;
                for chunk in chunk_refs.into_iter().take(max_chunks_per_entity.max(1)) {
                    let snippet = chunk.item.chunk.trim();
                    if snippet.is_empty() {
                        continue;
                    }
                    if !header_added {
                        doc.push_str("Chunks:\n");
                        header_added = true;
                    }
                    doc.push_str("- ");
                    doc.push_str(snippet);
                    doc.push('\n');
                }
            }

            doc
        })
        .collect()
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

fn apply_rerank_results(ctx: &mut PipelineContext<'_>, results: Vec<RerankResult>) {
    if results.is_empty() || ctx.filtered_entities.is_empty() {
        return;
    }

    let mut remaining: Vec<Option<Scored<KnowledgeEntity>>> =
        std::mem::take(&mut ctx.filtered_entities)
            .into_iter()
            .map(Some)
            .collect();

    let raw_scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let normalized_scores = min_max_normalize(&raw_scores);

    let use_only = ctx.config.tuning.rerank_scores_only;
    let blend = if use_only {
        1.0
    } else {
        clamp_unit(ctx.config.tuning.rerank_blend_weight)
    };
    let mut reranked: Vec<Scored<KnowledgeEntity>> = Vec::with_capacity(remaining.len());
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
                "Reranker returned out-of-range index; skipping"
            );
        }
        if reranked.len() == remaining.len() {
            break;
        }
    }

    for slot in remaining.into_iter() {
        if let Some(candidate) = slot {
            reranked.push(candidate);
        }
    }

    ctx.filtered_entities = reranked;
    let keep_top = ctx.config.tuning.rerank_keep_top;
    if keep_top > 0 && ctx.filtered_entities.len() > keep_top {
        ctx.filtered_entities.truncate(keep_top);
    }
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

    let use_only = tuning.rerank_scores_only;
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

    for slot in remaining.into_iter() {
        if let Some(candidate) = slot {
            reranked.push(candidate);
        }
    }

    let keep_top = tuning.rerank_keep_top;
    if keep_top > 0 && reranked.len() > keep_top {
        reranked.truncate(keep_top);
    }

    *chunks = reranked;
}

fn estimate_tokens(text: &str, avg_chars_per_token: usize) -> usize {
    let chars = text.chars().count().max(1);
    (chars / avg_chars_per_token).max(1)
}

fn rank_chunks_by_combined_score(
    candidates: &mut [Scored<TextChunk>],
    question_terms: &[String],
    lexical_weight: f32,
) {
    if lexical_weight > 0.0 && !question_terms.is_empty() {
        for candidate in candidates.iter_mut() {
            let lexical = lexical_overlap_score(question_terms, &candidate.item.chunk);
            let combined = clamp_unit(candidate.fused + lexical_weight * lexical);
            candidate.update_fused(combined);
        }
    }
    candidates.sort_by(|a, b| b.fused.partial_cmp(&a.fused).unwrap_or(Ordering::Equal));
}

fn extract_keywords(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let term = raw.trim().to_ascii_lowercase();
        if term.len() >= 3 {
            terms.push(term);
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn lexical_overlap_score(terms: &[String], haystack: &str) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let lower = haystack.to_ascii_lowercase();
    let mut matches = 0usize;
    for term in terms {
        if lower.contains(term) {
            matches += 1;
        }
    }
    (matches as f32) / (terms.len() as f32)
}

#[derive(Clone)]
struct GraphSeed {
    id: String,
    fused: f32,
}

fn seeds_from_candidates(
    entity_candidates: &HashMap<String, Scored<KnowledgeEntity>>,
    min_score: f32,
    limit: usize,
) -> Vec<GraphSeed> {
    let mut seeds: Vec<GraphSeed> = entity_candidates
        .values()
        .filter(|entity| entity.fused >= min_score)
        .map(|entity| GraphSeed {
            id: entity.item.id.clone(),
            fused: entity.fused,
        })
        .collect();

    seeds.sort_by(|a, b| {
        b.fused
            .partial_cmp(&a.fused)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if seeds.len() > limit {
        seeds.truncate(limit);
    }

    seeds
}
