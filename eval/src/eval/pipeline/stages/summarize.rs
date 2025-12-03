use std::time::Instant;

use chrono::Utc;
use tracing::info;

use crate::eval::{
    build_stage_latency_breakdown, compute_latency_stats, EvaluationSummary, PerformanceTimings,
};

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{EvaluationMachine, QueriesFinished, Summarized},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn summarize(
    machine: EvaluationMachine<(), QueriesFinished>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<Summarized> {
    let stage = EvalStage::Summarize;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    let summaries = std::mem::take(&mut ctx.query_summaries);
    let latencies = std::mem::take(&mut ctx.latencies);
    let stage_latency_samples = std::mem::take(&mut ctx.stage_latency_samples);
    let duration_ms = ctx
        .evaluation_start
        .take()
        .map(|start| start.elapsed().as_millis())
        .unwrap_or_default();
    let config = ctx.config();
    let dataset = ctx.dataset();
    let slice = ctx.slice();
    let corpus_handle = ctx.corpus_handle();
    let total_cases = summaries.len();

    let mut correct = 0usize;
    let mut correct_at_1 = 0usize;
    let mut correct_at_2 = 0usize;
    let mut correct_at_3 = 0usize;
    let mut retrieval_cases = 0usize;
    let mut llm_cases = 0usize;
    let mut llm_answered = 0usize;
    let mut sum_reciprocal_rank = 0.0;
    let mut sum_ndcg = 0.0;
    for summary in &summaries {
        if summary.is_impossible {
            llm_cases += 1;
            if summary.matched {
                llm_answered += 1;
            }
            continue;
        }
        retrieval_cases += 1;
        if let Some(rr) = summary.reciprocal_rank {
            sum_reciprocal_rank += rr;
        }
        if let Some(ndcg) = summary.ndcg {
            sum_ndcg += ndcg;
        }
        if summary.matched {
            correct += 1;
            if let Some(rank) = summary.match_rank {
                if rank <= 1 {
                    correct_at_1 += 1;
                }
                if rank <= 2 {
                    correct_at_2 += 1;
                }
                if rank <= 3 {
                    correct_at_3 += 1;
                }
            }
        }
    }

    let latency_stats = compute_latency_stats(&latencies);
    let stage_latency = build_stage_latency_breakdown(&stage_latency_samples);

    let retrieval_precision = if retrieval_cases == 0 {
        0.0
    } else {
        (correct as f64) / (retrieval_cases as f64)
    };
    let llm_precision = if llm_cases == 0 {
        0.0
    } else {
        (llm_answered as f64) / (llm_cases as f64)
    };
    let precision = retrieval_precision;
    let precision_at_1 = if retrieval_cases == 0 {
        0.0
    } else {
        (correct_at_1 as f64) / (retrieval_cases as f64)
    };
    let precision_at_2 = if retrieval_cases == 0 {
        0.0
    } else {
        (correct_at_2 as f64) / (retrieval_cases as f64)
    };
    let precision_at_3 = if retrieval_cases == 0 {
        0.0
    } else {
        (correct_at_3 as f64) / (retrieval_cases as f64)
    };
    let mrr = if retrieval_cases == 0 {
        0.0
    } else {
        sum_reciprocal_rank / (retrieval_cases as f64)
    };
    let average_ndcg = if retrieval_cases == 0 {
        0.0
    } else {
        sum_ndcg / (retrieval_cases as f64)
    };

    let active_tuning = ctx
        .retrieval_config
        .as_ref()
        .map(|cfg| cfg.tuning.clone())
        .unwrap_or_default();

    let perf_timings = PerformanceTimings {
        openai_base_url: ctx
            .openai_base_url
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string()),
        ingestion_ms: ctx.ingestion_duration_ms,
        namespace_seed_ms: ctx.namespace_seed_ms,
        evaluation_stage_ms: ctx.stage_timings.clone(),
        stage_latency,
    };

    ctx.summary = Some(EvaluationSummary {
        generated_at: Utc::now(),
        k: config.k,
        limit: config.limit,
        run_label: config.label.clone(),
        total_cases,
        correct,
        precision,
        correct_at_1,
        correct_at_2,
        correct_at_3,
        precision_at_1,
        precision_at_2,
        precision_at_3,
        mrr,
        average_ndcg,
        duration_ms,
        dataset_id: dataset.metadata.id.clone(),
        dataset_label: dataset.metadata.label.clone(),
        dataset_includes_unanswerable: dataset.metadata.include_unanswerable,
        dataset_source: dataset.source.clone(),
        includes_impossible_cases: slice.manifest.includes_unanswerable,
        require_verified_chunks: slice.manifest.require_verified_chunks,
        filtered_questions: ctx.filtered_questions,
        retrieval_cases,
        retrieval_correct: correct,
        retrieval_precision,
        llm_cases,
        llm_answered,
        llm_precision,
        slice_id: slice.manifest.slice_id.clone(),
        slice_seed: slice.manifest.seed,
        slice_total_cases: slice.manifest.case_count,
        slice_window_offset: ctx.window_offset,
        slice_window_length: ctx.window_length,
        slice_cases: total_cases,
        slice_positive_paragraphs: slice.manifest.positive_paragraphs,
        slice_negative_paragraphs: slice.manifest.negative_paragraphs,
        slice_total_paragraphs: slice.manifest.total_paragraphs,
        slice_negative_multiplier: slice.manifest.negative_multiplier,
        namespace_reused: ctx.namespace_reused,
        corpus_paragraphs: ctx.corpus_handle().manifest.metadata.paragraph_count,
        ingestion_cache_path: corpus_handle.path.display().to_string(),
        ingestion_reused: corpus_handle.reused_ingestion,
        ingestion_embeddings_reused: corpus_handle.reused_embeddings,
        ingestion_fingerprint: corpus_handle
            .manifest
            .metadata
            .ingestion_fingerprint
            .clone(),
        positive_paragraphs_reused: corpus_handle.positive_reused,
        negative_paragraphs_reused: corpus_handle.negative_reused,
        latency_ms: latency_stats,
        perf: perf_timings,
        embedding_backend: ctx.embedding_provider().backend_label().to_string(),
        embedding_model: ctx.embedding_provider().model_code(),
        embedding_dimension: ctx.embedding_provider().dimension(),
        rerank_enabled: config.retrieval.rerank,
        rerank_pool_size: ctx
            .rerank_pool
            .as_ref()
            .map(|_| config.retrieval.rerank_pool_size),
        rerank_keep_top: config.retrieval.rerank_keep_top,
        concurrency: config.concurrency.max(1),
        detailed_report: config.detailed_report,
        retrieval_strategy: config.retrieval.strategy.to_string(),
        chunk_result_cap: config.retrieval.chunk_result_cap,
        ingest_chunk_min_tokens: config.ingest_chunk_min_tokens,
        ingest_chunk_max_tokens: config.ingest_chunk_max_tokens,
        ingest_chunks_only: config.ingest_chunks_only,
        ingest_chunk_overlap_tokens: config.ingest_chunk_overlap_tokens,
        chunk_vector_take: active_tuning.chunk_vector_take,
        chunk_fts_take: active_tuning.chunk_fts_take,
        chunk_avg_chars_per_token: active_tuning.avg_chars_per_token,
        max_chunks_per_entity: active_tuning.max_chunks_per_entity,
        cases: summaries,
    });

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .summarize()
        .map_err(|(_, guard)| map_guard_error("summarize", guard))
}
