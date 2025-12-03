use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    args,
    eval::EvaluationSummary,
    report::{self, EvaluationReport},
};

pub fn mirror_perf_outputs(
    record: &EvaluationReport,
    summary: &EvaluationSummary,
    report_root: &Path,
    extra_json: Option<&Path>,
    extra_dir: Option<&Path>,
) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();

    if let Some(path) = extra_json {
        args::ensure_parent(path)?;
        let blob = serde_json::to_vec_pretty(record).context("serialising perf log JSON")?;
        fs::write(path, blob)
            .with_context(|| format!("writing perf log copy to {}", path.display()))?;
        written.push(path.to_path_buf());
    }

    if let Some(dir) = extra_dir {
        fs::create_dir_all(dir)
            .with_context(|| format!("creating perf log directory {}", dir.display()))?;
        let dataset_dir = report::dataset_report_dir(report_root, &summary.dataset_id);
        let dataset_slug = dataset_dir
            .file_name()
            .and_then(|os| os.to_str())
            .unwrap_or("dataset");
        let timestamp = summary.generated_at.format("%Y%m%dT%H%M%S").to_string();
        let filename = format!("perf-{}-{}.json", dataset_slug, timestamp);
        let path = dir.join(filename);
        let blob = serde_json::to_vec_pretty(record).context("serialising perf log JSON")?;
        fs::write(&path, blob)
            .with_context(|| format!("writing perf log mirror {}", path.display()))?;
        written.push(path);
    }

    Ok(written)
}

pub fn print_console_summary(record: &EvaluationReport) {
    let perf = &record.performance;
    println!(
        "[perf] retrieval strategy={} | concurrency={} | rerank={} (pool {:?}, keep {})",
        record.retrieval.strategy,
        record.retrieval.concurrency,
        record.retrieval.rerank_enabled,
        record.retrieval.rerank_pool_size,
        record.retrieval.rerank_keep_top
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
    let eval = &perf.evaluation_stages_ms;
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
            ingest_chunk_min_tokens: 256,
            ingest_chunk_max_tokens: 512,
            ingest_chunk_overlap_tokens: 50,
            ingest_chunks_only: false,
            chunk_vector_take: 20,
            chunk_fts_take: 20,
            chunk_avg_chars_per_token: 4,
            max_chunks_per_entity: 4,
            average_ndcg: 0.0,
            mrr: 0.0,
            cases: Vec::new(),
        }
    }

    #[test]
    fn writes_perf_mirrors_from_record() {
        let tmp = tempdir().unwrap();
        let report_root = tmp.path().join("reports");
        let summary = sample_summary();
        let record = report::EvaluationReport::from_summary(&summary, 5);

        let json_path = tmp.path().join("extra.json");
        let dir_path = tmp.path().join("copies");
        let outputs = mirror_perf_outputs(
            &record,
            &summary,
            &report_root,
            Some(json_path.as_path()),
            Some(dir_path.as_path()),
        )
        .expect("perf mirrors");

        assert!(json_path.exists());
        let content = std::fs::read_to_string(&json_path).expect("reading mirror json");
        assert!(
            content.contains("\"evaluation_stages_ms\""),
            "perf mirror should include evaluation stage timings"
        );
        assert_eq!(outputs.len(), 2);
        let mirrored = outputs
            .into_iter()
            .filter(|path| path.starts_with(&dir_path))
            .collect::<Vec<_>>();
        assert_eq!(mirrored.len(), 1, "expected timestamped mirror in dir");
    }
}
