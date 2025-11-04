use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::eval::{format_timestamp, CaseSummary, EvaluationSummary, LatencyStats};

#[derive(Debug)]
pub struct ReportPaths {
    pub json: PathBuf,
    pub markdown: PathBuf,
}

pub fn write_reports(
    summary: &EvaluationSummary,
    report_dir: &Path,
    sample: usize,
) -> Result<ReportPaths> {
    fs::create_dir_all(report_dir)
        .with_context(|| format!("creating report directory {}", report_dir.display()))?;
    let dataset_dir = dataset_report_dir(report_dir, &summary.dataset_id);
    fs::create_dir_all(&dataset_dir).with_context(|| {
        format!(
            "creating dataset report directory {}",
            dataset_dir.display()
        )
    })?;

    let stem = build_report_stem(summary);

    let json_path = dataset_dir.join(format!("{stem}.json"));
    let json_blob = serde_json::to_string_pretty(summary).context("serialising JSON report")?;
    fs::write(&json_path, &json_blob)
        .with_context(|| format!("writing JSON report to {}", json_path.display()))?;

    let md_path = dataset_dir.join(format!("{stem}.md"));
    let markdown = render_markdown(summary, sample);
    fs::write(&md_path, &markdown)
        .with_context(|| format!("writing Markdown report to {}", md_path.display()))?;

    // Keep a latest.json pointer to simplify automation.
    let latest_json = dataset_dir.join("latest.json");
    fs::write(&latest_json, json_blob)
        .with_context(|| format!("writing latest JSON report to {}", latest_json.display()))?;
    let latest_md = dataset_dir.join("latest.md");
    fs::write(&latest_md, markdown)
        .with_context(|| format!("writing latest Markdown report to {}", latest_md.display()))?;

    record_history(summary, &dataset_dir)?;

    Ok(ReportPaths {
        json: json_path,
        markdown: md_path,
    })
}

fn render_markdown(summary: &EvaluationSummary, sample: usize) -> String {
    let mut md = String::new();

    md.push_str(&format!("# Retrieval Precision@{}\n\n", summary.k));
    md.push_str("| Metric | Value |\n");
    md.push_str("| --- | --- |\n");
    md.push_str(&format!(
        "| Generated | {} |\n",
        format_timestamp(&summary.generated_at)
    ));
    md.push_str(&format!(
        "| Dataset | {} (`{}`) |\n",
        summary.dataset_label, summary.dataset_id
    ));
    md.push_str(&format!(
        "| Run Label | {} |\n",
        summary
            .run_label
            .as_deref()
            .filter(|label| !label.is_empty())
            .unwrap_or("-")
    ));
    md.push_str(&format!(
        "| Unanswerable Included | {} |\n",
        if summary.dataset_includes_unanswerable {
            "yes"
        } else {
            "no"
        }
    ));
    md.push_str(&format!(
        "| Dataset Source | {} |\n",
        summary.dataset_source
    ));
    md.push_str(&format!(
        "| OpenAI Base URL | {} |\n",
        summary.perf.openai_base_url
    ));
    md.push_str(&format!("| Slice ID | `{}` |\n", summary.slice_id));
    md.push_str(&format!("| Slice Seed | {} |\n", summary.slice_seed));
    md.push_str(&format!(
        "| Slice Total Questions | {} |\n",
        summary.slice_total_cases
    ));
    md.push_str(&format!(
        "| Slice Window (offset/length) | {}/{} |\n",
        summary.slice_window_offset, summary.slice_window_length
    ));
    md.push_str(&format!(
        "| Slice Window Questions | {} |\n",
        summary.slice_cases
    ));
    md.push_str(&format!(
        "| Slice Negatives | {} |\n",
        summary.slice_negative_paragraphs
    ));
    md.push_str(&format!(
        "| Slice Total Paragraphs | {} |\n",
        summary.slice_total_paragraphs
    ));
    md.push_str(&format!(
        "| Slice Negative Multiplier | {:.2} |\n",
        summary.slice_negative_multiplier
    ));
    md.push_str(&format!(
        "| Namespace State | {} |\n",
        if summary.namespace_reused {
            "reused"
        } else {
            "seeded"
        }
    ));
    md.push_str(&format!(
        "| Corpus Paragraphs | {} |\n",
        summary.corpus_paragraphs
    ));
    md.push_str(&format!(
        "| Ingestion Duration | {} ms |\n",
        summary.perf.ingestion_ms
    ));
    if let Some(seed) = summary.perf.namespace_seed_ms {
        md.push_str(&format!("| Namespace Seed | {} ms |\n", seed));
    }
    if summary.detailed_report {
        md.push_str(&format!(
            "| Ingestion Cache | `{}` |\n",
            summary.ingestion_cache_path
        ));
        md.push_str(&format!(
            "| Ingestion Reused | {} |\n",
            if summary.ingestion_reused {
                "yes"
            } else {
                "no"
            }
        ));
        md.push_str(&format!(
            "| Embeddings Reused | {} |\n",
            if summary.ingestion_embeddings_reused {
                "yes"
            } else {
                "no"
            }
        ));
    }
    md.push_str(&format!(
        "| Positives Cached | {} |
",
        summary.positive_paragraphs_reused
    ));
    md.push_str(&format!(
        "| Negatives Cached | {} |
",
        summary.negative_paragraphs_reused
    ));
    let embedding_label = if let Some(model) = summary.embedding_model.as_ref() {
        format!("{} ({model})", summary.embedding_backend)
    } else {
        summary.embedding_backend.clone()
    };
    md.push_str(&format!("| Embedding | {} |\n", embedding_label));
    md.push_str(&format!(
        "| Embedding Dim | {} |\n",
        summary.embedding_dimension
    ));
    if let Some(limit) = summary.limit {
        md.push_str(&format!(
            "| Evaluated Queries | {} (limit {}) |\n",
            summary.total_cases, limit
        ));
    } else {
        md.push_str(&format!(
            "| Evaluated Queries | {} |\n",
            summary.total_cases
        ));
    }
    if summary.rerank_enabled {
        let pool = summary
            .rerank_pool_size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "?".to_string());
        md.push_str(&format!(
            "| Rerank | enabled (pool {pool}, keep top {}) |\n",
            summary.rerank_keep_top
        ));
    } else {
        md.push_str("| Rerank | disabled |\n");
    }
    md.push_str(&format!("| Concurrency | {} |\n", summary.concurrency));
    md.push_str(&format!(
        "| Correct@{} | {}/{} |\n",
        summary.k, summary.correct, summary.total_cases
    ));
    md.push_str(&format!(
        "| Precision@{} | {:.3} |\n",
        summary.k, summary.precision
    ));
    md.push_str(&format!(
        "| Precision@1 | {:.3} |\n",
        summary.precision_at_1
    ));
    md.push_str(&format!(
        "| Precision@2 | {:.3} |\n",
        summary.precision_at_2
    ));
    md.push_str(&format!(
        "| Precision@3 | {:.3} |\n",
        summary.precision_at_3
    ));
    md.push_str(&format!("| Duration | {} ms |\n", summary.duration_ms));
    md.push_str(&format!(
        "| Latency Avg (ms) | {:.1} |\n",
        summary.latency_ms.avg
    ));
    md.push_str(&format!(
        "| Latency P50 (ms) | {} |\n",
        summary.latency_ms.p50
    ));
    md.push_str(&format!(
        "| Latency P95 (ms) | {} |\n",
        summary.latency_ms.p95
    ));

    md.push_str("\n## Retrieval Stage Timings\n\n");
    md.push_str("| Stage | Avg (ms) | P50 (ms) | P95 (ms) |\n");
    md.push_str("| --- | --- | --- | --- |\n");
    write_stage_row(
        &mut md,
        "Collect Candidates",
        &summary.perf.stage_latency.collect_candidates,
    );
    write_stage_row(
        &mut md,
        "Graph Expansion",
        &summary.perf.stage_latency.graph_expansion,
    );
    write_stage_row(
        &mut md,
        "Chunk Attach",
        &summary.perf.stage_latency.chunk_attach,
    );
    write_stage_row(&mut md, "Rerank", &summary.perf.stage_latency.rerank);
    write_stage_row(&mut md, "Assemble", &summary.perf.stage_latency.assemble);

    let misses: Vec<&CaseSummary> = summary.cases.iter().filter(|case| !case.matched).collect();
    if !misses.is_empty() {
        md.push_str("\n## Missed Queries (sample)\n\n");
        if summary.detailed_report {
            md.push_str(
                "| Question ID | Paragraph | Expected Source | Entity Match | Chunk Text | Chunk ID | Top Retrieved |\n",
            );
            md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        } else {
            md.push_str("| Question ID | Paragraph | Expected Source | Top Retrieved |\n");
            md.push_str("| --- | --- | --- | --- |\n");
        }

        for case in misses.iter().take(sample) {
            let retrieved = case
                .retrieved
                .iter()
                .map(|entry| format!("{} (rank {})", entry.source_id, entry.rank))
                .take(3)
                .collect::<Vec<_>>()
                .join("<br>");
            if summary.detailed_report {
                md.push_str(&format!(
                    "| `{}` | {} | `{}` | {} | {} | {} | {} |\n",
                    case.question_id,
                    case.paragraph_title,
                    case.expected_source,
                    bool_badge(case.entity_match),
                    bool_badge(case.chunk_text_match),
                    bool_badge(case.chunk_id_match),
                    retrieved
                ));
            } else {
                md.push_str(&format!(
                    "| `{}` | {} | `{}` | {} |\n",
                    case.question_id, case.paragraph_title, case.expected_source, retrieved
                ));
            }
        }
    } else {
        md.push_str("\n_All evaluated queries matched within the top-k window._\n");
        if summary.detailed_report {
            md.push_str(
                "\nSuccess measures were captured for each query (entity, chunk text, chunk ID).\n",
            );
        }
    }

    md
}

fn write_stage_row(buf: &mut String, label: &str, stats: &LatencyStats) {
    buf.push_str(&format!(
        "| {} | {:.1} | {} | {} |\n",
        label, stats.avg, stats.p50, stats.p95
    ));
}

fn bool_badge(value: bool) -> &'static str {
    if value {
        "✅"
    } else {
        "⚪"
    }
}

fn build_report_stem(summary: &EvaluationSummary) -> String {
    let timestamp = summary.generated_at.format("%Y%m%dT%H%M%S");
    let backend = sanitize_component(&summary.embedding_backend);
    let dataset_component = sanitize_component(&summary.dataset_id);
    let model_component = summary
        .embedding_model
        .as_ref()
        .map(|model| sanitize_component(model));

    match model_component {
        Some(model) => format!(
            "precision_at_{}_{}_{}_{}_{}",
            summary.k, dataset_component, timestamp, backend, model
        ),
        None => format!(
            "precision_at_{}_{}_{}_{}",
            summary.k, dataset_component, timestamp, backend
        ),
    }
}

fn sanitize_component(input: &str) -> String {
    input
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

pub fn dataset_report_dir(report_dir: &Path, dataset_id: &str) -> PathBuf {
    report_dir.join(sanitize_component(dataset_id))
}

#[derive(Debug, Serialize, Deserialize)]
struct HistoryEntry {
    generated_at: String,
    run_label: Option<String>,
    dataset_id: String,
    dataset_label: String,
    slice_id: String,
    slice_seed: u64,
    slice_window_offset: usize,
    slice_window_length: usize,
    slice_cases: usize,
    slice_total_cases: usize,
    k: usize,
    limit: Option<usize>,
    precision: f64,
    precision_at_1: f64,
    precision_at_2: f64,
    precision_at_3: f64,
    duration_ms: u128,
    latency_ms: LatencyStats,
    embedding_backend: String,
    embedding_model: Option<String>,
    ingestion_reused: bool,
    ingestion_embeddings_reused: bool,
    rerank_enabled: bool,
    rerank_keep_top: usize,
    rerank_pool_size: Option<usize>,
    delta: Option<HistoryDelta>,
    openai_base_url: String,
    ingestion_ms: u128,
    #[serde(default)]
    namespace_seed_ms: Option<u128>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HistoryDelta {
    precision: f64,
    precision_at_1: f64,
    latency_avg_ms: f64,
}

fn record_history(summary: &EvaluationSummary, report_dir: &Path) -> Result<()> {
    let path = report_dir.join("evaluations.json");
    let mut entries: Vec<HistoryEntry> = if path.exists() {
        let contents = fs::read(&path)
            .with_context(|| format!("reading evaluation log {}", path.display()))?;
        serde_json::from_slice(&contents).unwrap_or_default()
    } else {
        Vec::new()
    };

    let delta = entries.last().map(|prev| HistoryDelta {
        precision: summary.precision - prev.precision,
        precision_at_1: summary.precision_at_1 - prev.precision_at_1,
        latency_avg_ms: summary.latency_ms.avg - prev.latency_ms.avg,
    });

    let entry = HistoryEntry {
        generated_at: format_timestamp(&summary.generated_at),
        run_label: summary.run_label.clone(),
        dataset_id: summary.dataset_id.clone(),
        dataset_label: summary.dataset_label.clone(),
        slice_id: summary.slice_id.clone(),
        slice_seed: summary.slice_seed,
        slice_window_offset: summary.slice_window_offset,
        slice_window_length: summary.slice_window_length,
        slice_cases: summary.slice_cases,
        slice_total_cases: summary.slice_total_cases,
        k: summary.k,
        limit: summary.limit,
        precision: summary.precision,
        precision_at_1: summary.precision_at_1,
        precision_at_2: summary.precision_at_2,
        precision_at_3: summary.precision_at_3,
        duration_ms: summary.duration_ms,
        latency_ms: summary.latency_ms.clone(),
        embedding_backend: summary.embedding_backend.clone(),
        embedding_model: summary.embedding_model.clone(),
        ingestion_reused: summary.ingestion_reused,
        ingestion_embeddings_reused: summary.ingestion_embeddings_reused,
        rerank_enabled: summary.rerank_enabled,
        rerank_keep_top: summary.rerank_keep_top,
        rerank_pool_size: summary.rerank_pool_size,
        delta,
        openai_base_url: summary.perf.openai_base_url.clone(),
        ingestion_ms: summary.perf.ingestion_ms,
        namespace_seed_ms: summary.perf.namespace_seed_ms,
    };

    entries.push(entry);

    let blob = serde_json::to_vec_pretty(&entries).context("serialising evaluation log")?;
    fs::write(&path, blob).with_context(|| format!("writing evaluation log {}", path.display()))?;
    Ok(())
}
