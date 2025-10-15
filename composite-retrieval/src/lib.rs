pub mod answer_retrieval;
pub mod answer_retrieval_helper;
pub mod fts;
pub mod graph;
pub mod scoring;
pub mod vector;

use std::collections::{HashMap, HashSet};

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk, StoredObject},
    },
    utils::embedding::generate_embedding,
};
use futures::{stream::FuturesUnordered, StreamExt};
use graph::{find_entities_by_relationship_by_id, find_entities_by_source_ids};
use scoring::{
    clamp_unit, fuse_scores, merge_scored_by_id, min_max_normalize, sort_by_fused_desc,
    FusionWeights, Scored,
};
use tracing::{debug, instrument, trace};

use crate::{fts::find_items_by_fts, vector::find_items_by_vector_similarity_with_embedding};

// Tunable knobs controlling first-pass recall, graph expansion, and answer shaping.
const ENTITY_VECTOR_TAKE: usize = 15;
const CHUNK_VECTOR_TAKE: usize = 20;
const ENTITY_FTS_TAKE: usize = 10;
const CHUNK_FTS_TAKE: usize = 20;
const SCORE_THRESHOLD: f32 = 0.35;
const FALLBACK_MIN_RESULTS: usize = 10;
const TOKEN_BUDGET_ESTIMATE: usize = 2800;
const AVG_CHARS_PER_TOKEN: usize = 4;
const MAX_CHUNKS_PER_ENTITY: usize = 4;
const GRAPH_TRAVERSAL_SEED_LIMIT: usize = 5;
const GRAPH_NEIGHBOR_LIMIT: usize = 6;
const GRAPH_SCORE_DECAY: f32 = 0.75;
const GRAPH_SEED_MIN_SCORE: f32 = 0.4;
const GRAPH_VECTOR_INHERITANCE: f32 = 0.6;

// Captures a supporting chunk plus its fused retrieval score for downstream prompts.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub chunk: TextChunk,
    pub score: f32,
}

// Final entity representation returned to callers, enriched with ranked chunks.
#[derive(Debug, Clone)]
pub struct RetrievedEntity {
    pub entity: KnowledgeEntity,
    pub score: f32,
    pub chunks: Vec<RetrievedChunk>,
}

#[instrument(skip_all, fields(user_id))]
pub async fn retrieve_entities(
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    query: &str,
    user_id: &str,
) -> Result<Vec<RetrievedEntity>, AppError> {
    trace!("Generating query embedding for hybrid retrieval");
    let query_embedding = generate_embedding(openai_client, query, db_client).await?;
    retrieve_entities_with_embedding(db_client, query_embedding, query, user_id).await
}

pub(crate) async fn retrieve_entities_with_embedding(
    db_client: &SurrealDbClient,
    query_embedding: Vec<f32>,
    query: &str,
    user_id: &str,
) -> Result<Vec<RetrievedEntity>, AppError> {
    // 1) Gather first-pass candidates from vector search and BM25.
    let weights = FusionWeights::default();

    let (vector_entities, vector_chunks, mut fts_entities, mut fts_chunks) = tokio::try_join!(
        find_items_by_vector_similarity_with_embedding(
            ENTITY_VECTOR_TAKE,
            query_embedding.clone(),
            db_client,
            "knowledge_entity",
            user_id,
        ),
        find_items_by_vector_similarity_with_embedding(
            CHUNK_VECTOR_TAKE,
            query_embedding,
            db_client,
            "text_chunk",
            user_id,
        ),
        find_items_by_fts(
            ENTITY_FTS_TAKE,
            query,
            db_client,
            "knowledge_entity",
            user_id
        ),
        find_items_by_fts(CHUNK_FTS_TAKE, query, db_client, "text_chunk", user_id),
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

    let mut entity_candidates: HashMap<String, Scored<KnowledgeEntity>> = HashMap::new();
    let mut chunk_candidates: HashMap<String, Scored<TextChunk>> = HashMap::new();

    // Collate raw retrieval results so each ID accumulates all available signals.
    merge_scored_by_id(&mut entity_candidates, vector_entities);
    merge_scored_by_id(&mut entity_candidates, fts_entities);
    merge_scored_by_id(&mut chunk_candidates, vector_chunks);
    merge_scored_by_id(&mut chunk_candidates, fts_chunks);

    // 2) Normalize scores, fuse them, and allow high-confidence entities to pull neighbors from the graph.
    apply_fusion(&mut entity_candidates, weights);
    apply_fusion(&mut chunk_candidates, weights);
    enrich_entities_from_graph(&mut entity_candidates, db_client, user_id, weights).await?;

    // 3) Track high-signal chunk sources so we can backfill missing entities.
    let chunk_by_source = group_chunks_by_source(&chunk_candidates);
    let mut missing_sources = Vec::new();

    for source_id in chunk_by_source.keys() {
        if !entity_candidates
            .values()
            .any(|entity| entity.item.source_id == *source_id)
        {
            missing_sources.push(source_id.clone());
        }
    }

    if !missing_sources.is_empty() {
        let related_entities: Vec<KnowledgeEntity> = find_entities_by_source_ids(
            missing_sources.clone(),
            "knowledge_entity",
            user_id,
            db_client,
        )
        .await
        .unwrap_or_default();

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
    }

    // Boost entities with evidence from high scoring chunks.
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

    let mut entity_results: Vec<Scored<KnowledgeEntity>> =
        entity_candidates.into_values().collect();
    sort_by_fused_desc(&mut entity_results);

    let mut filtered_entities: Vec<Scored<KnowledgeEntity>> = entity_results
        .iter()
        .filter(|candidate| candidate.fused >= SCORE_THRESHOLD)
        .cloned()
        .collect();

    if filtered_entities.len() < FALLBACK_MIN_RESULTS {
        // Low recall scenarios still benefit from some context; take the top N regardless of score.
        filtered_entities = entity_results
            .into_iter()
            .take(FALLBACK_MIN_RESULTS)
            .collect();
    }

    // 4) Re-rank chunks and prepare for attachment to surviving entities.
    let mut chunk_results: Vec<Scored<TextChunk>> = chunk_candidates.into_values().collect();
    sort_by_fused_desc(&mut chunk_results);

    let mut chunk_by_id: HashMap<String, Scored<TextChunk>> = HashMap::new();
    for chunk in chunk_results {
        chunk_by_id.insert(chunk.item.id.clone(), chunk);
    }

    enrich_chunks_from_entities(
        &mut chunk_by_id,
        &filtered_entities,
        db_client,
        user_id,
        weights,
    )
    .await?;

    let mut chunk_values: Vec<Scored<TextChunk>> = chunk_by_id.into_values().collect();
    sort_by_fused_desc(&mut chunk_values);

    Ok(assemble_results(filtered_entities, chunk_values))
}

// Minimal record used while seeding graph expansion so we can retain the original fused score.
#[derive(Clone)]
struct GraphSeed {
    id: String,
    fused: f32,
}

async fn enrich_entities_from_graph(
    entity_candidates: &mut HashMap<String, Scored<KnowledgeEntity>>,
    db_client: &SurrealDbClient,
    user_id: &str,
    weights: FusionWeights,
) -> Result<(), AppError> {
    if entity_candidates.is_empty() {
        return Ok(());
    }

    // Select a small frontier of high-confidence entities to seed the relationship walk.
    let mut seeds: Vec<GraphSeed> = entity_candidates
        .values()
        .filter(|entity| entity.fused >= GRAPH_SEED_MIN_SCORE)
        .map(|entity| GraphSeed {
            id: entity.item.id.clone(),
            fused: entity.fused,
        })
        .collect();

    if seeds.is_empty() {
        return Ok(());
    }

    // Prioritise the strongest seeds so we explore the most grounded context first.
    seeds.sort_by(|a, b| {
        b.fused
            .partial_cmp(&a.fused)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    seeds.truncate(GRAPH_TRAVERSAL_SEED_LIMIT);

    let mut futures = FuturesUnordered::new();
    for seed in seeds.clone() {
        let user_id = user_id.to_owned();
        futures.push(async move {
            // Fetch neighbors concurrently to avoid serial graph round trips.
            let neighbors = find_entities_by_relationship_by_id(
                db_client,
                &seed.id,
                &user_id,
                GRAPH_NEIGHBOR_LIMIT,
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

        // Fold neighbors back into the candidate map and let them inherit attenuated signal.
        for neighbor in neighbors {
            if neighbor.id == seed.id {
                continue;
            }

            let graph_score = clamp_unit(seed.fused * GRAPH_SCORE_DECAY);
            let entry = entity_candidates
                .entry(neighbor.id.clone())
                .or_insert_with(|| Scored::new(neighbor.clone()));

            entry.item = neighbor;

            let inherited_vector = clamp_unit(graph_score * GRAPH_VECTOR_INHERITANCE);
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

fn normalize_fts_scores<T>(results: &mut [Scored<T>]) {
    // Scale BM25 outputs into [0,1] to keep fusion weights predictable.
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
    // Collapse individual signals into a single fused score used for ranking.
    for candidate in candidates.values_mut() {
        let fused = fuse_scores(&candidate.scores, weights);
        candidate.update_fused(fused);
    }
}

fn group_chunks_by_source(
    chunks: &HashMap<String, Scored<TextChunk>>,
) -> HashMap<String, Vec<Scored<TextChunk>>> {
    // Preserve chunk candidates keyed by their originating source entity.
    let mut by_source: HashMap<String, Vec<Scored<TextChunk>>> = HashMap::new();

    for chunk in chunks.values() {
        by_source
            .entry(chunk.item.source_id.clone())
            .or_default()
            .push(chunk.clone());
    }
    by_source
}

async fn enrich_chunks_from_entities(
    chunk_candidates: &mut HashMap<String, Scored<TextChunk>>,
    entities: &[Scored<KnowledgeEntity>],
    db_client: &SurrealDbClient,
    user_id: &str,
    weights: FusionWeights,
) -> Result<(), AppError> {
    // Fetch additional chunks referenced by entities that survived the fusion stage.
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
    // Cache fused scores per source so chunks inherit the strength of their parent entity.
    for entity in entities {
        entity_score_lookup.insert(entity.item.source_id.clone(), entity.fused);
    }

    for chunk in chunks {
        // Ensure each chunk is represented so downstream selection sees the latest content.
        let entry = chunk_candidates
            .entry(chunk.id.clone())
            .or_insert_with(|| Scored::new(chunk.clone()).with_vector_score(0.0));

        let entity_score = entity_score_lookup
            .get(&chunk.source_id)
            .copied()
            .unwrap_or(0.0);

        // Lift chunk score toward the entity score so supporting evidence is prioritised.
        entry.scores.vector = Some(entry.scores.vector.unwrap_or(0.0).max(entity_score * 0.8));
        let fused = fuse_scores(&entry.scores, weights);
        entry.update_fused(fused);
        entry.item = chunk;
    }

    Ok(())
}

fn assemble_results(
    entities: Vec<Scored<KnowledgeEntity>>,
    mut chunks: Vec<Scored<TextChunk>>,
) -> Vec<RetrievedEntity> {
    // Re-associate chunk candidates with their parent entity for ranked selection.
    let mut chunk_by_source: HashMap<String, Vec<Scored<TextChunk>>> = HashMap::new();
    for chunk in chunks.drain(..) {
        chunk_by_source
            .entry(chunk.item.source_id.clone())
            .or_default()
            .push(chunk);
    }

    for chunk_list in chunk_by_source.values_mut() {
        sort_by_fused_desc(chunk_list);
    }

    let mut token_budget_remaining = TOKEN_BUDGET_ESTIMATE;
    let mut results = Vec::new();

    for entity in entities {
        // Attach best chunks first while respecting per-entity and global token caps.
        let mut selected_chunks = Vec::new();
        if let Some(candidates) = chunk_by_source.get_mut(&entity.item.source_id) {
            let mut per_entity_count = 0;
            candidates.sort_by(|a, b| {
                b.fused
                    .partial_cmp(&a.fused)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for candidate in candidates.iter() {
                if per_entity_count >= MAX_CHUNKS_PER_ENTITY {
                    break;
                }
                let estimated_tokens = estimate_tokens(&candidate.item.chunk);
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

    results
}

fn estimate_tokens(text: &str) -> usize {
    // Simple heuristic to avoid calling a tokenizer in hot code paths.
    let chars = text.chars().count().max(1);
    (chars / AVG_CHARS_PER_TOKEN).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::storage::types::{
        knowledge_entity::KnowledgeEntityType, knowledge_relationship::KnowledgeRelationship,
    };
    use uuid::Uuid;

    fn test_embedding() -> Vec<f32> {
        vec![0.9, 0.1, 0.0]
    }

    fn entity_embedding_high() -> Vec<f32> {
        vec![0.8, 0.2, 0.0]
    }

    fn entity_embedding_low() -> Vec<f32> {
        vec![0.1, 0.9, 0.0]
    }

    fn chunk_embedding_primary() -> Vec<f32> {
        vec![0.85, 0.15, 0.0]
    }

    fn chunk_embedding_secondary() -> Vec<f32> {
        vec![0.2, 0.8, 0.0]
    }

    async fn setup_test_db() -> SurrealDbClient {
        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("Failed to start in-memory surrealdb");

        db.apply_migrations()
            .await
            .expect("Failed to apply migrations");

        db.query(
            "BEGIN TRANSACTION;
             REMOVE INDEX IF EXISTS idx_embedding_chunks ON TABLE text_chunk;
             DEFINE INDEX idx_embedding_chunks ON TABLE text_chunk FIELDS embedding HNSW DIMENSION 3;
             REMOVE INDEX IF EXISTS idx_embedding_entities ON TABLE knowledge_entity;
             DEFINE INDEX idx_embedding_entities ON TABLE knowledge_entity FIELDS embedding HNSW DIMENSION 3;
             COMMIT TRANSACTION;",
        )
        .await
        .expect("Failed to redefine vector indexes for tests");

        db
    }

    async fn seed_test_data(db: &SurrealDbClient, user_id: &str) {
        let entity_relevant = KnowledgeEntity::new(
            "source_a".into(),
            "Rust Concurrency Patterns".into(),
            "Discussion about async concurrency in Rust.".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_high(),
            user_id.into(),
        );
        let entity_irrelevant = KnowledgeEntity::new(
            "source_b".into(),
            "Python Tips".into(),
            "General Python programming tips.".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_low(),
            user_id.into(),
        );

        db.store_item(entity_relevant.clone())
            .await
            .expect("Failed to store relevant entity");
        db.store_item(entity_irrelevant.clone())
            .await
            .expect("Failed to store irrelevant entity");

        let chunk_primary = TextChunk::new(
            entity_relevant.source_id.clone(),
            "Tokio enables async concurrency with lightweight tasks.".into(),
            chunk_embedding_primary(),
            user_id.into(),
        );
        let chunk_secondary = TextChunk::new(
            entity_irrelevant.source_id.clone(),
            "Python focuses on readability and dynamic typing.".into(),
            chunk_embedding_secondary(),
            user_id.into(),
        );

        db.store_item(chunk_primary)
            .await
            .expect("Failed to store primary chunk");
        db.store_item(chunk_secondary)
            .await
            .expect("Failed to store secondary chunk");
    }

    #[tokio::test]
    async fn test_hybrid_retrieval_prioritises_relevant_entity() {
        let db = setup_test_db().await;
        let user_id = "user123";
        seed_test_data(&db, user_id).await;

        let results = retrieve_entities_with_embedding(
            &db,
            test_embedding(),
            "Rust concurrency async tasks",
            user_id,
        )
        .await
        .expect("Hybrid retrieval failed");

        assert!(
            !results.is_empty(),
            "Expected at least one retrieval result"
        );

        let top = &results[0];

        assert!(
            top.entity.name.contains("Rust"),
            "Expected Rust entity to be ranked first"
        );
        assert!(
            !top.chunks.is_empty(),
            "Expected Rust entity to include supporting chunks"
        );

        let chunk_texts: Vec<&str> = top
            .chunks
            .iter()
            .map(|chunk| chunk.chunk.chunk.as_str())
            .collect();
        assert!(
            chunk_texts.iter().any(|text| text.contains("Tokio")),
            "Expected chunk discussing Tokio to be included"
        );
    }

    #[tokio::test]
    async fn test_graph_relationship_enriches_results() {
        let db = setup_test_db().await;
        let user_id = "graph_user";

        let primary = KnowledgeEntity::new(
            "primary_source".into(),
            "Async Rust patterns".into(),
            "Explores async runtimes and scheduling strategies.".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_high(),
            user_id.into(),
        );
        let neighbor = KnowledgeEntity::new(
            "neighbor_source".into(),
            "Tokio Scheduler Deep Dive".into(),
            "Details on Tokio's cooperative scheduler.".into(),
            KnowledgeEntityType::Document,
            None,
            entity_embedding_low(),
            user_id.into(),
        );

        db.store_item(primary.clone())
            .await
            .expect("Failed to store primary entity");
        db.store_item(neighbor.clone())
            .await
            .expect("Failed to store neighbor entity");

        let primary_chunk = TextChunk::new(
            primary.source_id.clone(),
            "Rust async tasks use Tokio's cooperative scheduler.".into(),
            chunk_embedding_primary(),
            user_id.into(),
        );
        let neighbor_chunk = TextChunk::new(
            neighbor.source_id.clone(),
            "Tokio's scheduler manages task fairness across executors.".into(),
            chunk_embedding_secondary(),
            user_id.into(),
        );

        db.store_item(primary_chunk)
            .await
            .expect("Failed to store primary chunk");
        db.store_item(neighbor_chunk)
            .await
            .expect("Failed to store neighbor chunk");

        let relationship = KnowledgeRelationship::new(
            primary.id.clone(),
            neighbor.id.clone(),
            user_id.into(),
            "relationship_source".into(),
            "references".into(),
        );
        relationship
            .store_relationship(&db)
            .await
            .expect("Failed to store relationship");

        let results = retrieve_entities_with_embedding(
            &db,
            test_embedding(),
            "Rust concurrency async tasks",
            user_id,
        )
        .await
        .expect("Hybrid retrieval failed");

        let mut neighbor_entry = None;
        for entity in &results {
            if entity.entity.id == neighbor.id {
                neighbor_entry = Some(entity.clone());
            }
        }

        let neighbor_entry =
            neighbor_entry.expect("Graph-enriched neighbor should appear in results");

        assert!(
            neighbor_entry.score > 0.2,
            "Graph-enriched entity should have a meaningful fused score"
        );
        assert!(
            neighbor_entry
                .chunks
                .iter()
                .all(|chunk| chunk.chunk.source_id == neighbor.source_id),
            "Neighbor entity should surface its own supporting chunks"
        );
    }
}
