use std::{collections::HashSet, sync::Arc, time::Instant};

use anyhow::Context;
use futures::stream::{self, StreamExt, TryStreamExt};
use tracing::{debug, info};

use crate::eval::{
    apply_dataset_tuning_overrides, build_case_diagnostics, text_contains_answer, CaseDiagnostics,
    CaseSummary, RetrievedSummary,
};
use composite_retrieval::pipeline::{self, PipelineStageTimings, RetrievalConfig};
use composite_retrieval::reranking::RerankerPool;
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

    let rerank_pool = if config.rerank {
        Some(RerankerPool::new(config.rerank_pool_size).context("initialising reranker pool")?)
    } else {
        None
    };

    let mut retrieval_config = RetrievalConfig::default();
    retrieval_config.tuning.rerank_keep_top = config.rerank_keep_top;
    if retrieval_config.tuning.fallback_min_results < config.rerank_keep_top {
        retrieval_config.tuning.fallback_min_results = config.rerank_keep_top;
    }
    if let Some(value) = config.chunk_vector_take {
        retrieval_config.tuning.chunk_vector_take = value;
    }
    if let Some(value) = config.chunk_fts_take {
        retrieval_config.tuning.chunk_fts_take = value;
    }
    if let Some(value) = config.chunk_token_budget {
        retrieval_config.tuning.token_budget_estimate = value;
    }
    if let Some(value) = config.chunk_avg_chars_per_token {
        retrieval_config.tuning.avg_chars_per_token = value;
    }
    if let Some(value) = config.max_chunks_per_entity {
        retrieval_config.tuning.max_chunks_per_entity = value;
    }

    apply_dataset_tuning_overrides(dataset, config, &mut retrieval_config.tuning);

    let active_tuning = retrieval_config.tuning.clone();
    let effective_chunk_vector = config
        .chunk_vector_take
        .unwrap_or(active_tuning.chunk_vector_take);
    let effective_chunk_fts = config
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
        rerank_enabled = config.rerank,
        rerank_pool_size = config.rerank_pool_size,
        rerank_keep_top = config.rerank_keep_top,
        chunk_min = config.chunk_min_chars,
        chunk_max = config.chunk_max_chars,
        chunk_vector_take = effective_chunk_vector,
        chunk_fts_take = effective_chunk_fts,
        chunk_token_budget = active_tuning.token_budget_estimate,
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

    let results: Vec<(
        usize,
        CaseSummary,
        Option<CaseDiagnostics>,
        PipelineStageTimings,
    )> = stream::iter(cases_iter)
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

                let (results, pipeline_diagnostics, stage_timings) = if diagnostics_enabled {
                    let outcome = pipeline::run_pipeline_with_embedding_with_diagnostics(
                        &db,
                        &openai_client,
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

                let mut retrieved = Vec::new();
                let mut match_rank = None;
                let answers_lower: Vec<String> =
                    answers.iter().map(|ans| ans.to_ascii_lowercase()).collect();
                let expected_chunk_ids_set: HashSet<&str> =
                    expected_chunk_ids.iter().map(|id| id.as_str()).collect();
                let chunk_id_required = !expected_chunk_ids_set.is_empty();
                let mut entity_hit = false;
                let mut chunk_text_hit = false;
                let mut chunk_id_hit = !chunk_id_required;

                for (idx_entity, entity) in results.iter().enumerate() {
                    if idx_entity >= config.k {
                        break;
                    }
                    let entity_match = entity.entity.source_id == expected_source;
                    if entity_match {
                        entity_hit = true;
                    }
                    let chunk_text_for_entity = entity
                        .chunks
                        .iter()
                        .any(|chunk| text_contains_answer(&chunk.chunk.chunk, &answers_lower));
                    if chunk_text_for_entity {
                        chunk_text_hit = true;
                    }
                    let chunk_id_for_entity = if chunk_id_required {
                        expected_chunk_ids_set.contains(entity.entity.source_id.as_str())
                            || entity.chunks.iter().any(|chunk| {
                                expected_chunk_ids_set.contains(chunk.chunk.id.as_str())
                            })
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
                        (
                            Some(entity.entity.description.clone()),
                            Some(format!("{:?}", entity.entity.entity_type)),
                            Some(chunk_text_for_entity),
                            Some(chunk_id_for_entity),
                        )
                    } else {
                        (None, None, None, None)
                    };
                    retrieved.push(RetrievedSummary {
                        rank: idx_entity + 1,
                        entity_id: entity.entity.id.clone(),
                        source_id: entity.entity.source_id.clone(),
                        entity_name: entity.entity.name.clone(),
                        score: entity.score,
                        matched: success,
                        entity_description: detail_fields.0,
                        entity_category: detail_fields.1,
                        chunk_text_match: detail_fields.2,
                        chunk_id_match: detail_fields.3,
                    });
                }

                let overall_match = match_rank.is_some();
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
                    match_rank,
                    latency_ms: query_latency,
                    retrieved,
                };

                let diagnostics = if diagnostics_enabled {
                    Some(build_case_diagnostics(
                        &summary,
                        &expected_chunk_ids,
                        &answers_lower,
                        &results,
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
        .try_collect()
        .await?;

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
