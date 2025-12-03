use std::{collections::HashSet, sync::Arc, time::Instant};

use anyhow::Context;
use common::storage::types::StoredObject;
use futures::stream::{self, StreamExt};
use tracing::{debug, info};

use crate::eval::{
    adapt_strategy_output, build_case_diagnostics,
    text_contains_answer, CaseDiagnostics, CaseSummary, RetrievedSummary,
};
use retrieval_pipeline::{
    pipeline::{self, PipelineStageTimings, RetrievalConfig},
    reranking::RerankerPool,
};
use tokio::sync::Semaphore;

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{EvaluationMachine, NamespaceReady, QueriesFinished},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn run_queries(
    machine: EvaluationMachine<(), NamespaceReady>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<QueriesFinished> {
    let stage = EvalStage::RunQueries;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    let config = ctx.config();
    let dataset = ctx.dataset();
    let slice_settings = ctx
        .slice_settings
        .as_ref()
        .expect("slice settings missing during query stage");
    let total_cases = ctx.cases.len();
    let cases_iter = std::mem::take(&mut ctx.cases).into_iter().enumerate();

    let rerank_pool = if config.retrieval.rerank {
        Some(
            RerankerPool::new(config.retrieval.rerank_pool_size)
                .context("initialising reranker pool")?,
        )
    } else {
        None
    };

    let mut retrieval_config = RetrievalConfig::default();
    retrieval_config.strategy = config.retrieval.strategy;
    retrieval_config.tuning.rerank_keep_top = config.retrieval.rerank_keep_top;
    if retrieval_config.tuning.fallback_min_results < config.retrieval.rerank_keep_top {
        retrieval_config.tuning.fallback_min_results = config.retrieval.rerank_keep_top;
    }
    retrieval_config.tuning.chunk_result_cap = config.retrieval.chunk_result_cap.max(1);
    if let Some(value) = config.retrieval.chunk_vector_take {
        retrieval_config.tuning.chunk_vector_take = value;
    }
    if let Some(value) = config.retrieval.chunk_fts_take {
        retrieval_config.tuning.chunk_fts_take = value;
    }
    if let Some(value) = config.retrieval.chunk_avg_chars_per_token {
        retrieval_config.tuning.avg_chars_per_token = value;
    }
    if let Some(value) = config.retrieval.max_chunks_per_entity {
        retrieval_config.tuning.max_chunks_per_entity = value;
    }

    let active_tuning = retrieval_config.tuning.clone();
    let effective_chunk_vector = config
        .retrieval
        .chunk_vector_take
        .unwrap_or(active_tuning.chunk_vector_take);
    let effective_chunk_fts = config
        .retrieval
        .chunk_fts_take
        .unwrap_or(active_tuning.chunk_fts_take);

    info!(
        dataset = dataset.metadata.id.as_str(),
        slice_seed = config.slice_seed,
        slice_offset = config.slice_offset,
        slice_limit = config
            .limit
            .unwrap_or(ctx.window_total_cases),
        negative_multiplier = %slice_settings.negative_multiplier,
        rerank_enabled = config.retrieval.rerank,
        rerank_pool_size = config.retrieval.rerank_pool_size,
        rerank_keep_top = config.retrieval.rerank_keep_top,
        chunk_vector_take = effective_chunk_vector,
        chunk_fts_take = effective_chunk_fts,
        embedding_backend = ctx.embedding_provider().backend_label(),
        embedding_model = ctx
            .embedding_provider()
            .model_code()
            .as_deref()
            .unwrap_or("<default>"),
        "Starting evaluation run"
    );

    let retrieval_config = Arc::new(retrieval_config);
    ctx.rerank_pool = rerank_pool.clone();
    ctx.retrieval_config = Some(retrieval_config.clone());

    ctx.evaluation_start = Some(Instant::now());
    let user_id = ctx.evaluation_user().id.clone();
    let concurrency = config.concurrency.max(1);
    let diagnostics_enabled = ctx.diagnostics_enabled;

    let query_semaphore = Arc::new(Semaphore::new(concurrency));

    info!(
        total_cases = total_cases,
        max_concurrent_queries = concurrency,
        "Starting evaluation with staged query execution"
    );

    let embedding_provider_for_queries = ctx.embedding_provider().clone();
    let rerank_pool_for_queries = rerank_pool.clone();
    let db = ctx.db().clone();
    let openai_client = ctx.openai_client();

    let raw_results = stream::iter(cases_iter)
        .map(move |(idx, case)| {
            let db = db.clone();
            let openai_client = openai_client.clone();
            let user_id = user_id.clone();
            let retrieval_config = retrieval_config.clone();
            let embedding_provider = embedding_provider_for_queries.clone();
            let rerank_pool = rerank_pool_for_queries.clone();
            let semaphore = query_semaphore.clone();
            let diagnostics_enabled = diagnostics_enabled;

            async move {
                let _permit = semaphore
                    .acquire()
                    .await
                    .context("acquiring query semaphore permit")?;

                let crate::eval::SeededCase {
                    question_id,
                    question,
                    expected_source,
                    answers,
                    paragraph_id,
                    paragraph_title,
                    expected_chunk_ids,
                    is_impossible,
                    has_verified_chunks,
                } = case;
                let query_start = Instant::now();

                debug!(question_id = %question_id, "Evaluating query");
                let query_embedding =
                    embedding_provider.embed(&question).await.with_context(|| {
                        format!("generating embedding for question {}", question_id)
                    })?;
                let reranker = match &rerank_pool {
                    Some(pool) => Some(pool.checkout().await),
                    None => None,
                };

                let (result_output, pipeline_diagnostics, stage_timings) = if diagnostics_enabled {
                    let outcome = pipeline::run_pipeline_with_embedding_with_diagnostics(
                        &db,
                        &openai_client,
                        Some(&embedding_provider),
                        query_embedding,
                        &question,
                        &user_id,
                        (*retrieval_config).clone(),
                        reranker,
                    )
                    .await
                    .with_context(|| format!("running pipeline for question {}", question_id))?;
                    (outcome.results, outcome.diagnostics, outcome.stage_timings)
                } else {
                    let outcome = pipeline::run_pipeline_with_embedding_with_metrics(
                        &db,
                        &openai_client,
                        Some(&embedding_provider),
                        query_embedding,
                        &question,
                        &user_id,
                        (*retrieval_config).clone(),
                        reranker,
                    )
                    .await
                    .with_context(|| format!("running pipeline for question {}", question_id))?;
                    (outcome.results, None, outcome.stage_timings)
                };
                let query_latency = query_start.elapsed().as_millis() as u128;

                let candidates = adapt_strategy_output(result_output);
                let mut retrieved = Vec::new();
                let mut match_rank = None;
                let answers_lower: Vec<String> =
                    answers.iter().map(|ans| ans.to_ascii_lowercase()).collect();
                let expected_chunk_ids_set: HashSet<&str> =
                    expected_chunk_ids.iter().map(|id| id.as_str()).collect();
                let chunk_id_required = has_verified_chunks;
                let mut entity_hit = false;
                let mut chunk_text_hit = false;
                let mut chunk_id_hit = !chunk_id_required;

                for (idx_entity, candidate) in candidates.iter().enumerate() {
                    if idx_entity >= config.k {
                        break;
                    }
                    let entity_match = candidate.source_id == expected_source;
                    if entity_match {
                        entity_hit = true;
                    }
                    let chunk_text_for_entity = candidate
                        .chunks
                        .iter()
                        .any(|chunk| text_contains_answer(&chunk.chunk.chunk, &answers_lower));
                    if chunk_text_for_entity {
                        chunk_text_hit = true;
                    }
                    let chunk_id_for_entity = if chunk_id_required {
                        expected_chunk_ids_set.contains(candidate.source_id.as_str())
                            || candidate
                                .chunks
                                .iter()
                                .any(|chunk| expected_chunk_ids_set.contains(&chunk.chunk.get_id()))
                    } else {
                        true
                    };
                    if chunk_id_for_entity {
                        chunk_id_hit = true;
                    }
                    let success = entity_match && chunk_text_for_entity && chunk_id_for_entity;
                    if success && match_rank.is_none() {
                        match_rank = Some(idx_entity + 1);
                    }
                    let detail_fields = if config.detailed_report {
                        let description = candidate.entity_description.clone();
                        let category = candidate.entity_category.clone();
                        (
                            description,
                            category,
                            Some(chunk_text_for_entity),
                            Some(chunk_id_for_entity),
                        )
                    } else {
                        (None, None, None, None)
                    };
                    retrieved.push(RetrievedSummary {
                        rank: idx_entity + 1,
                        entity_id: candidate.entity_id.clone(),
                        source_id: candidate.source_id.clone(),
                        entity_name: candidate.entity_name.clone(),
                        score: candidate.score,
                        matched: success,
                        entity_description: detail_fields.0,
                        entity_category: detail_fields.1,
                        chunk_text_match: detail_fields.2,
                        chunk_id_match: detail_fields.3,
                    });
                }

                let overall_match = match_rank.is_some();
                let reciprocal_rank = calculate_reciprocal_rank(match_rank);
                let ndcg = calculate_ndcg(&retrieved, config.k);

                let summary = CaseSummary {
                    question_id,
                    question,
                    paragraph_id,
                    paragraph_title,
                    expected_source,
                    answers,
                    matched: overall_match,
                    entity_match: entity_hit,
                    chunk_text_match: chunk_text_hit,
                    chunk_id_match: chunk_id_hit,
                    is_impossible,
                    has_verified_chunks,
                    match_rank,
                    reciprocal_rank: Some(reciprocal_rank),
                    ndcg: Some(ndcg),
                    latency_ms: query_latency,
                    retrieved,
                };

                let diagnostics = if diagnostics_enabled {
                    Some(build_case_diagnostics(
                        &summary,
                        &expected_chunk_ids,
                        &answers_lower,
                        &candidates,
                        pipeline_diagnostics,
                    ))
                } else {
                    None
                };

                Ok::<
                    (
                        usize,
                        CaseSummary,
                        Option<CaseDiagnostics>,
                        PipelineStageTimings,
                    ),
                    anyhow::Error,
                >((idx, summary, diagnostics, stage_timings))
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;

    let mut results = Vec::with_capacity(raw_results.len());
    for result in raw_results {
        match result {
            Ok(val) => results.push(val),
            Err(err) => {
                tracing::error!(error = ?err, "Query execution failed");
            }
        }
    }

    let mut ordered = results;
    ordered.sort_by_key(|(idx, ..)| *idx);
    let mut summaries = Vec::with_capacity(ordered.len());
    let mut latencies = Vec::with_capacity(ordered.len());
    let mut diagnostics_output = Vec::new();
    let mut stage_latency_samples = Vec::with_capacity(ordered.len());
    for (_, summary, diagnostics, stage_timings) in ordered {
        latencies.push(summary.latency_ms);
        summaries.push(summary);
        if let Some(diag) = diagnostics {
            diagnostics_output.push(diag);
        }
        stage_latency_samples.push(stage_timings);
    }

    ctx.query_summaries = summaries;
    ctx.latencies = latencies;
    ctx.diagnostics_output = diagnostics_output;
    ctx.stage_latency_samples = stage_latency_samples;

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .run_queries()
        .map_err(|(_, guard)| map_guard_error("run_queries", guard))
}

fn calculate_reciprocal_rank(rank: Option<usize>) -> f64 {
    match rank {
        Some(r) if r > 0 => 1.0 / (r as f64),
        _ => 0.0,
    }
}

fn calculate_ndcg(retrieved: &[RetrievedSummary], k: usize) -> f64 {
    let mut dcg = 0.0;
    let mut relevant_count = 0;

    for (i, item) in retrieved.iter().enumerate() {
        if i >= k {
            break;
        }
        if item.matched {
            let rel = 1.0;
            dcg += rel / (i as f64 + 2.0).log2();
            relevant_count += 1;
        }
    }

    if dcg == 0.0 {
        return 0.0;
    }

    // Calculate IDCG based on the number of relevant items found
    // We assume ideal ordering would place all 'relevant_count' items at the top
    let mut idcg = 0.0;
    for i in 0..relevant_count {
        let rel = 1.0;
        idcg += rel / (i as f64 + 2.0).log2();
    }

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}
