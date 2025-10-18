use async_openai::Client;
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk, StoredObject},
    },
    utils::embedding::generate_embedding,
};
use futures::{stream::FuturesUnordered, StreamExt};
use state_machines::core::GuardError;
use std::collections::{HashMap, HashSet};
use tracing::{debug, instrument, warn};

use crate::{
    fts::find_items_by_fts,
    graph::{find_entities_by_relationship_by_id, find_entities_by_source_ids},
    scoring::{
        clamp_unit, fuse_scores, merge_scored_by_id, min_max_normalize, sort_by_fused_desc,
        FusionWeights, Scored,
    },
    vector::find_items_by_vector_similarity_with_embedding,
    RetrievedChunk, RetrievedEntity,
};

use super::{
    config::RetrievalConfig,
    state::{
        CandidatesLoaded, ChunksAttached, Embedded, GraphExpanded, HybridRetrievalMachine, Ready,
    },
};

pub struct PipelineContext<'a> {
    pub db_client: &'a SurrealDbClient,
    pub openai_client: &'a Client<async_openai::config::OpenAIConfig>,
    pub input_text: String,
    pub user_id: String,
    pub config: RetrievalConfig,
    pub query_embedding: Option<Vec<f32>>,
    pub entity_candidates: HashMap<String, Scored<KnowledgeEntity>>,
    pub chunk_candidates: HashMap<String, Scored<TextChunk>>,
    pub filtered_entities: Vec<Scored<KnowledgeEntity>>,
    pub chunk_values: Vec<Scored<TextChunk>>,
}

impl<'a> PipelineContext<'a> {
    pub fn new(
        db_client: &'a SurrealDbClient,
        openai_client: &'a Client<async_openai::config::OpenAIConfig>,
        input_text: String,
        user_id: String,
        config: RetrievalConfig,
    ) -> Self {
        Self {
            db_client,
            openai_client,
            input_text,
            user_id,
            config,
            query_embedding: None,
            entity_candidates: HashMap::new(),
            chunk_candidates: HashMap::new(),
            filtered_entities: Vec::new(),
            chunk_values: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn with_embedding(
        db_client: &'a SurrealDbClient,
        openai_client: &'a Client<async_openai::config::OpenAIConfig>,
        query_embedding: Vec<f32>,
        input_text: String,
        user_id: String,
        config: RetrievalConfig,
    ) -> Self {
        let mut ctx = Self::new(db_client, openai_client, input_text, user_id, config);
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
}

#[instrument(level = "trace", skip_all)]
pub async fn embed(
    machine: HybridRetrievalMachine<(), Ready>,
    ctx: &mut PipelineContext<'_>,
) -> Result<HybridRetrievalMachine<(), Embedded>, AppError> {
    let embedding_cached = ctx.query_embedding.is_some();
    if embedding_cached {
        debug!("Reusing cached query embedding for hybrid retrieval");
    } else {
        debug!("Generating query embedding for hybrid retrieval");
        let embedding =
            generate_embedding(ctx.openai_client, &ctx.input_text, ctx.db_client).await?;
        ctx.query_embedding = Some(embedding);
    }

    machine
        .embed()
        .map_err(|(_, guard)| map_guard_error("embed", guard))
}

#[instrument(level = "trace", skip_all)]
pub async fn collect_candidates(
    machine: HybridRetrievalMachine<(), Embedded>,
    ctx: &mut PipelineContext<'_>,
) -> Result<HybridRetrievalMachine<(), CandidatesLoaded>, AppError> {
    debug!("Collecting initial candidates via vector and FTS search");
    let embedding = ctx.ensure_embedding()?.clone();
    let tuning = &ctx.config.tuning;

    let weights = FusionWeights::default();

    let (vector_entities, vector_chunks, mut fts_entities, mut fts_chunks) = tokio::try_join!(
        find_items_by_vector_similarity_with_embedding(
            tuning.entity_vector_take,
            embedding.clone(),
            ctx.db_client,
            "knowledge_entity",
            &ctx.user_id,
        ),
        find_items_by_vector_similarity_with_embedding(
            tuning.chunk_vector_take,
            embedding,
            ctx.db_client,
            "text_chunk",
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

    debug!(
        vector_entities = vector_entities.len(),
        vector_chunks = vector_chunks.len(),
        fts_entities = fts_entities.len(),
        fts_chunks = fts_chunks.len(),
        "Hybrid retrieval initial candidate counts"
    );

    normalize_fts_scores(&mut fts_entities);
    normalize_fts_scores(&mut fts_chunks);

    merge_scored_by_id(&mut ctx.entity_candidates, vector_entities);
    merge_scored_by_id(&mut ctx.entity_candidates, fts_entities);
    merge_scored_by_id(&mut ctx.chunk_candidates, vector_chunks);
    merge_scored_by_id(&mut ctx.chunk_candidates, fts_chunks);

    apply_fusion(&mut ctx.entity_candidates, weights);
    apply_fusion(&mut ctx.chunk_candidates, weights);

    machine
        .collect_candidates()
        .map_err(|(_, guard)| map_guard_error("collect_candidates", guard))
}

#[instrument(level = "trace", skip_all)]
pub async fn expand_graph(
    machine: HybridRetrievalMachine<(), CandidatesLoaded>,
    ctx: &mut PipelineContext<'_>,
) -> Result<HybridRetrievalMachine<(), GraphExpanded>, AppError> {
    debug!("Expanding candidates using graph relationships");
    let tuning = &ctx.config.tuning;
    let weights = FusionWeights::default();

    if ctx.entity_candidates.is_empty() {
        return machine
            .expand_graph()
            .map_err(|(_, guard)| map_guard_error("expand_graph", guard));
    }

    let graph_seeds = seeds_from_candidates(
        &ctx.entity_candidates,
        tuning.graph_seed_min_score,
        tuning.graph_traversal_seed_limit,
    );

    if graph_seeds.is_empty() {
        return machine
            .expand_graph()
            .map_err(|(_, guard)| map_guard_error("expand_graph", guard));
    }

    let mut futures = FuturesUnordered::new();
    for seed in graph_seeds {
        let db = ctx.db_client;
        let user = ctx.user_id.clone();
        futures.push(async move {
            let neighbors = find_entities_by_relationship_by_id(
                db,
                &seed.id,
                &user,
                tuning.graph_neighbor_limit,
            )
            .await;
            (seed, neighbors)
        });
    }

    while let Some((seed, neighbors_result)) = futures.next().await {
        let neighbors = neighbors_result.map_err(AppError::from)?;
        if neighbors.is_empty() {
            continue;
        }

        for neighbor in neighbors {
            if neighbor.id == seed.id {
                continue;
            }

            let graph_score = clamp_unit(seed.fused * tuning.graph_score_decay);
            let entry = ctx
                .entity_candidates
                .entry(neighbor.id.clone())
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

    machine
        .expand_graph()
        .map_err(|(_, guard)| map_guard_error("expand_graph", guard))
}

#[instrument(level = "trace", skip_all)]
pub async fn attach_chunks(
    machine: HybridRetrievalMachine<(), GraphExpanded>,
    ctx: &mut PipelineContext<'_>,
) -> Result<HybridRetrievalMachine<(), ChunksAttached>, AppError> {
    debug!("Attaching chunks to surviving entities");
    let tuning = &ctx.config.tuning;
    let weights = FusionWeights::default();

    let chunk_by_source = group_chunks_by_source(&ctx.chunk_candidates);

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

    ctx.chunk_values = chunk_values;

    machine
        .attach_chunks()
        .map_err(|(_, guard)| map_guard_error("attach_chunks", guard))
}

#[instrument(level = "trace", skip_all)]
pub fn assemble(
    machine: HybridRetrievalMachine<(), ChunksAttached>,
    ctx: &mut PipelineContext<'_>,
) -> Result<Vec<RetrievedEntity>, AppError> {
    debug!("Assembling final retrieved entities");
    let tuning = &ctx.config.tuning;

    let mut chunk_by_source: HashMap<String, Vec<Scored<TextChunk>>> = HashMap::new();
    for chunk in ctx.chunk_values.drain(..) {
        chunk_by_source
            .entry(chunk.item.source_id.clone())
            .or_default()
            .push(chunk);
    }

    for chunk_list in chunk_by_source.values_mut() {
        sort_by_fused_desc(chunk_list);
    }

    let mut token_budget_remaining = tuning.token_budget_estimate;
    let mut results = Vec::new();

    for entity in &ctx.filtered_entities {
        let mut selected_chunks = Vec::new();
        if let Some(candidates) = chunk_by_source.get_mut(&entity.item.source_id) {
            let mut per_entity_count = 0;
            candidates.sort_by(|a, b| {
                b.fused
                    .partial_cmp(&a.fused)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for candidate in candidates.iter() {
                if per_entity_count >= tuning.max_chunks_per_entity {
                    break;
                }
                let estimated_tokens =
                    estimate_tokens(&candidate.item.chunk, tuning.avg_chars_per_token);
                if estimated_tokens > token_budget_remaining {
                    continue;
                }

                token_budget_remaining = token_budget_remaining.saturating_sub(estimated_tokens);
                per_entity_count += 1;

                selected_chunks.push(RetrievedChunk {
                    chunk: candidate.item.clone(),
                    score: candidate.fused,
                });
            }
        }

        results.push(RetrievedEntity {
            entity: entity.item.clone(),
            score: entity.fused,
            chunks: selected_chunks,
        });

        if token_budget_remaining == 0 {
            break;
        }
    }

    machine
        .assemble()
        .map_err(|(_, guard)| map_guard_error("assemble", guard))?;
    Ok(results)
}

fn map_guard_error(stage: &'static str, err: GuardError) -> AppError {
    AppError::InternalError(format!(
        "state machine guard '{stage}' failed: guard={}, event={}, kind={:?}",
        err.guard, err.event, err.kind
    ))
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

fn estimate_tokens(text: &str, avg_chars_per_token: usize) -> usize {
    let chars = text.chars().count().max(1);
    (chars / avg_chars_per_token).max(1)
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
