use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{
    args,
    eval::{format_timestamp, EvaluationStageTimings, EvaluationSummary},
    report,
};

#[derive(Debug, Serialize)]
struct PerformanceLogEntry {
    generated_at: String,
    dataset_id: String,
    dataset_label: String,
    run_label: Option<String>,
    retrieval_strategy: String,
    slice_id: String,
    slice_seed: u64,
    slice_window_offset: usize,
    slice_window_length: usize,
    limit: Option<usize>,
    total_cases: usize,
    correct: usize,
    precision: f64,
    retrieval_cases: usize,
    llm_cases: usize,
    llm_answered: usize,
    llm_precision: f64,
    k: usize,
    openai_base_url: String,
    ingestion: IngestionPerf,
    namespace: NamespacePerf,
    retrieval: RetrievalPerf,
    evaluation_stages: EvaluationStageTimings,
}

#[derive(Debug, Serialize)]
struct IngestionPerf {
    duration_ms: u128,
    cache_path: String,
    reused: bool,
    embeddings_reused: bool,
    fingerprint: String,
    positives_total: usize,
    negatives_total: usize,
}

#[derive(Debug, Serialize)]
struct NamespacePerf {
    reused: bool,
    seed_ms: Option<u128>,
}

#[derive(Debug, Serialize)]
struct RetrievalPerf {
    latency_ms: crate::eval::LatencyStats,
    stage_latency: crate::eval::StageLatencyBreakdown,
    concurrency: usize,
    rerank_enabled: bool,
    rerank_pool_size: Option<usize>,
    rerank_keep_top: usize,
    evaluated_cases: usize,
}

impl PerformanceLogEntry {
    fn from_summary(summary: &EvaluationSummary) -> Self {
        let ingestion = IngestionPerf {
            duration_ms: summary.perf.ingestion_ms,
            cache_path: summary.ingestion_cache_path.clone(),
            reused: summary.ingestion_reused,
            embeddings_reused: summary.ingestion_embeddings_reused,
            fingerprint: summary.ingestion_fingerprint.clone(),
            positives_total: summary.slice_positive_paragraphs,
            negatives_total: summary.slice_negative_paragraphs,
        };

        let namespace = NamespacePerf {
            reused: summary.namespace_reused,
            seed_ms: summary.perf.namespace_seed_ms,
        };

        let retrieval = RetrievalPerf {
            latency_ms: summary.latency_ms.clone(),
            stage_latency: summary.perf.stage_latency.clone(),
            concurrency: summary.concurrency,
            rerank_enabled: summary.rerank_enabled,
            rerank_pool_size: summary.rerank_pool_size,
            rerank_keep_top: summary.rerank_keep_top,
            evaluated_cases: summary.retrieval_cases,
        };

        Self {
            generated_at: format_timestamp(&summary.generated_at),
            dataset_id: summary.dataset_id.clone(),
            dataset_label: summary.dataset_label.clone(),
            run_label: summary.run_label.clone(),
            retrieval_strategy: summary.retrieval_strategy.clone(),
            slice_id: summary.slice_id.clone(),
            slice_seed: summary.slice_seed,
            slice_window_offset: summary.slice_window_offset,
            slice_window_length: summary.slice_window_length,
            limit: summary.limit,
            total_cases: summary.total_cases,
            correct: summary.correct,
            precision: summary.precision,
            retrieval_cases: summary.retrieval_cases,
            llm_cases: summary.llm_cases,
            llm_answered: summary.llm_answered,
            llm_precision: summary.llm_precision,
            k: summary.k,
            openai_base_url: summary.perf.openai_base_url.clone(),
            ingestion,
            namespace,
            retrieval,
            evaluation_stages: summary.perf.evaluation_stage_ms.clone(),
        }
    }
}

pub fn write_perf_logs(
    summary: &EvaluationSummary,
    report_root: &Path,
    extra_json: Option<&Path>,
    extra_dir: Option<&Path>,
) -> Result<PathBuf> {
    let entry = PerformanceLogEntry::from_summary(summary);
    let dataset_dir = report::dataset_report_dir(report_root, &summary.dataset_id);
    fs::create_dir_all(&dataset_dir)
        .with_context(|| format!("creating dataset perf directory {}", dataset_dir.display()))?;

    let log_path = dataset_dir.join("perf-log.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening perf log {}", log_path.display()))?;
    let line = serde_json::to_vec(&entry).context("serialising perf log entry")?;
    file.write_all(&line)?;
    file.write_all(b"\n")?;
    file.flush()?;

    if let Some(path) = extra_json {
        args::ensure_parent(path)?;
        let blob = serde_json::to_vec_pretty(&entry).context("serialising perf log JSON")?;
        fs::write(path, blob)
            .with_context(|| format!("writing perf log copy to {}", path.display()))?;
    }

    if let Some(dir) = extra_dir {
        fs::create_dir_all(dir)
            .with_context(|| format!("creating perf log directory {}", dir.display()))?;
        let dataset_slug = dataset_dir
            .file_name()
            .and_then(|os| os.to_str())
            .unwrap_or("dataset");
        let timestamp = summary.generated_at.format("%Y%m%dT%H%M%S").to_string();
        let filename = format!("perf-{}-{}.json", dataset_slug, timestamp);
        let path = dir.join(filename);
        let blob = serde_json::to_vec_pretty(&entry).context("serialising perf log JSON")?;
        fs::write(&path, blob)
            .with_context(|| format!("writing perf log mirror {}", path.display()))?;
    }

    Ok(log_path)
}

pub fn print_console_summary(summary: &EvaluationSummary) {
    let perf = &summary.perf;
    println!(
        "[perf] retrieval strategy={} | rerank={} (pool {:?}, keep {})",
        summary.retrieval_strategy,
        summary.rerank_enabled,
        summary.rerank_pool_size,
        summary.rerank_keep_top
    );
    println!(
        "[perf] ingestion={}ms | namespace_seed={}",
        perf.ingestion_ms,
        format_duration(perf.namespace_seed_ms),
    );
    let stage = &perf.stage_latency;
    println!(
        "[perf] stage avg ms → embed {:.1} | collect {:.1} | graph {:.1} | chunk {:.1} | rerank {:.1} | assemble {:.1}",
        stage.embed.avg,
        stage.collect_candidates.avg,
        stage.graph_expansion.avg,
        stage.chunk_attach.avg,
        stage.rerank.avg,
        stage.assemble.avg,
    );
    let eval = &perf.evaluation_stage_ms;
    println!(
        "[perf] eval stage ms → slice {} | db {} | corpus {} | namespace {} | queries {} | summarize {} | finalize {}",
        eval.prepare_slice_ms,
        eval.prepare_db_ms,
        eval.prepare_corpus_ms,
        eval.prepare_namespace_ms,
        eval.run_queries_ms,
        eval.summarize_ms,
        eval.finalize_ms,
    );
}

fn format_duration(value: Option<u128>) -> String {
    value
        .map(|ms| format!("{ms}ms"))
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{EvaluationStageTimings, PerformanceTimings};
    use chrono::Utc;
    use tempfile::tempdir;

    fn sample_latency() -> crate::eval::LatencyStats {
        crate::eval::LatencyStats {
            avg: 10.0,
            p50: 8,
            p95: 15,
        }
    }

    fn sample_stage_latency() -> crate::eval::StageLatencyBreakdown {
        crate::eval::StageLatencyBreakdown {
            embed: sample_latency(),
            collect_candidates: sample_latency(),
            graph_expansion: sample_latency(),
            chunk_attach: sample_latency(),
            rerank: sample_latency(),
            assemble: sample_latency(),
        }
    }

    fn sample_eval_stage() -> EvaluationStageTimings {
        EvaluationStageTimings {
            prepare_slice_ms: 10,
            prepare_db_ms: 20,
            prepare_corpus_ms: 30,
            prepare_namespace_ms: 40,
            run_queries_ms: 50,
            summarize_ms: 60,
            finalize_ms: 70,
        }
    }

    fn sample_summary() -> EvaluationSummary {
        EvaluationSummary {
            generated_at: Utc::now(),
            k: 5,
            limit: Some(10),
            run_label: Some("test".into()),
            total_cases: 2,
            correct: 1,
            precision: 0.5,
            correct_at_1: 1,
            correct_at_2: 1,
            correct_at_3: 1,
            precision_at_1: 0.5,
            precision_at_2: 0.5,
            precision_at_3: 0.5,
            duration_ms: 1234,
            dataset_id: "squad-v2".into(),
            dataset_label: "SQuAD v2".into(),
            dataset_includes_unanswerable: false,
            dataset_source: "dev".into(),
            includes_impossible_cases: false,
            require_verified_chunks: true,
            filtered_questions: 0,
            retrieval_cases: 2,
            retrieval_correct: 1,
            retrieval_precision: 0.5,
            llm_cases: 0,
            llm_answered: 0,
            llm_precision: 0.0,
            slice_id: "slice123".into(),
            slice_seed: 42,
            slice_total_cases: 400,
            slice_window_offset: 0,
            slice_window_length: 10,
            slice_cases: 10,
            slice_positive_paragraphs: 10,
            slice_negative_paragraphs: 40,
            slice_total_paragraphs: 50,
            slice_negative_multiplier: 4.0,
            namespace_reused: true,
            corpus_paragraphs: 50,
            ingestion_cache_path: "/tmp/cache".into(),
            ingestion_reused: true,
            ingestion_embeddings_reused: true,
            ingestion_fingerprint: "fingerprint".into(),
            positive_paragraphs_reused: 10,
            negative_paragraphs_reused: 40,
            latency_ms: sample_latency(),
            perf: PerformanceTimings {
                openai_base_url: "https://example.com".into(),
                ingestion_ms: 1000,
                namespace_seed_ms: Some(150),
                evaluation_stage_ms: sample_eval_stage(),
                stage_latency: sample_stage_latency(),
            },
            embedding_backend: "fastembed".into(),
            embedding_model: Some("test-model".into()),
            embedding_dimension: 32,
            rerank_enabled: true,
            rerank_pool_size: Some(4),
            rerank_keep_top: 10,
            concurrency: 2,
            retrieval_strategy: "initial".into(),
            detailed_report: false,
            chunk_vector_take: 20,
            chunk_fts_take: 20,
            chunk_token_budget: 10000,
            chunk_avg_chars_per_token: 4,
            max_chunks_per_entity: 4,
            cases: Vec::new(),
        }
    }

    #[test]
    fn writes_perf_log_jsonl() {
        let tmp = tempdir().unwrap();
        let report_root = tmp.path().join("reports");
        let summary = sample_summary();
        let log_path = write_perf_logs(&summary, &report_root, None, None).expect("perf log write");
        assert!(log_path.exists());
        let contents = std::fs::read_to_string(&log_path).expect("reading perf log jsonl");
        assert!(
            contents.contains("\"openai_base_url\":\"https://example.com\""),
            "serialized log should include base URL"
        );
        let dataset_dir = report::dataset_report_dir(&report_root, &summary.dataset_id);
        assert!(dataset_dir.join("perf-log.jsonl").exists());
    }
}
