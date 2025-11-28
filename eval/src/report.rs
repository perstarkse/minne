use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::eval::{
    format_timestamp, CaseSummary, EvaluationStageTimings, EvaluationSummary, LatencyStats,
    StageLatencyBreakdown,
};
use chrono::Utc;
use tracing::warn;

#[derive(Debug)]
pub struct ReportPaths {
    pub json: PathBuf,
    pub markdown: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    pub overview: OverviewSection,
    pub dataset: DatasetSection,
    pub slice: SliceSection,
    pub retrieval: RetrievalSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmSection>,
    pub performance: PerformanceSection,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub misses: Vec<MissEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub llm_cases: Vec<LlmCaseEntry>,
    pub detailed_report: bool,
}

#[derive(Debug, Serialize)]
pub struct OverviewSection {
    pub generated_at: String,
    pub run_label: Option<String>,
    pub total_cases: usize,
    pub filtered_questions: usize,
}

#[derive(Debug, Serialize)]
pub struct DatasetSection {
    pub id: String,
    pub label: String,
    pub source: String,
    pub includes_unanswerable: bool,
    pub require_verified_chunks: bool,
    pub embedding_backend: String,
    pub embedding_model: Option<String>,
    pub embedding_dimension: usize,
}

#[derive(Debug, Serialize)]
pub struct SliceSection {
    pub id: String,
    pub seed: u64,
    pub window_offset: usize,
    pub window_length: usize,
    pub slice_cases: usize,
    pub ledger_total_cases: usize,
    pub positives: usize,
    pub negatives: usize,
    pub total_paragraphs: usize,
    pub negative_multiplier: f32,
}

#[derive(Debug, Serialize)]
pub struct RetrievalSection {
    pub k: usize,
    pub cases: usize,
    pub correct: usize,
    pub precision: f64,
    pub precision_at_1: f64,
    pub precision_at_2: f64,
    pub precision_at_3: f64,
    pub mrr: f64,
    pub average_ndcg: f64,
    pub latency: LatencyStats,
    pub concurrency: usize,
    pub strategy: String,
    pub rerank_enabled: bool,
    pub rerank_pool_size: Option<usize>,
    pub rerank_keep_top: usize,
}

#[derive(Debug, Serialize)]
pub struct LlmSection {
    pub cases: usize,
    pub answered: usize,
    pub precision: f64,
}

#[derive(Debug, Serialize)]
pub struct PerformanceSection {
    pub openai_base_url: String,
    pub ingestion_ms: u128,
    pub namespace_seed_ms: Option<u128>,
    pub evaluation_stages_ms: EvaluationStageTimings,
    pub stage_latency: StageLatencyBreakdown,
    pub namespace_reused: bool,
    pub ingestion_reused: bool,
    pub embeddings_reused: bool,
    pub ingestion_cache_path: String,
    pub corpus_paragraphs: usize,
    pub positive_paragraphs_reused: usize,
    pub negative_paragraphs_reused: usize,
}

#[derive(Debug, Serialize)]
pub struct MissEntry {
    pub question_id: String,
    pub paragraph_title: String,
    pub expected_source: String,
    pub entity_match: bool,
    pub chunk_text_match: bool,
    pub chunk_id_match: bool,
    pub retrieved: Vec<RetrievedSnippet>,
}

#[derive(Debug, Serialize)]
pub struct LlmCaseEntry {
    pub question_id: String,
    pub answered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_rank: Option<usize>,
    pub retrieved: Vec<RetrievedSnippet>,
}

#[derive(Debug, Serialize)]
pub struct RetrievedSnippet {
    pub rank: usize,
    pub source_id: String,
    pub entity_name: String,
    pub matched: bool,
}

impl EvaluationReport {
    pub fn from_summary(summary: &EvaluationSummary, sample: usize) -> Self {
        let overview = OverviewSection {
            generated_at: format_timestamp(&summary.generated_at),
            run_label: summary.run_label.clone(),
            total_cases: summary.total_cases,
            filtered_questions: summary.filtered_questions,
        };

        let dataset = DatasetSection {
            id: summary.dataset_id.clone(),
            label: summary.dataset_label.clone(),
            source: summary.dataset_source.clone(),
            includes_unanswerable: summary.includes_impossible_cases,
            require_verified_chunks: summary.require_verified_chunks,
            embedding_backend: summary.embedding_backend.clone(),
            embedding_model: summary.embedding_model.clone(),
            embedding_dimension: summary.embedding_dimension,
        };

        let slice = SliceSection {
            id: summary.slice_id.clone(),
            seed: summary.slice_seed,
            window_offset: summary.slice_window_offset,
            window_length: summary.slice_window_length,
            slice_cases: summary.slice_cases,
            ledger_total_cases: summary.slice_total_cases,
            positives: summary.slice_positive_paragraphs,
            negatives: summary.slice_negative_paragraphs,
            total_paragraphs: summary.slice_total_paragraphs,
            negative_multiplier: summary.slice_negative_multiplier,
        };

        let retrieval = RetrievalSection {
            k: summary.k,
            cases: summary.retrieval_cases,
            correct: summary.retrieval_correct,
            precision: summary.retrieval_precision,
            precision_at_1: summary.precision_at_1,
            precision_at_2: summary.precision_at_2,
            precision_at_3: summary.precision_at_3,
            mrr: summary.mrr,
            average_ndcg: summary.average_ndcg,
            latency: summary.latency_ms.clone(),
            concurrency: summary.concurrency,
            strategy: summary.retrieval_strategy.clone(),
            rerank_enabled: summary.rerank_enabled,
            rerank_pool_size: summary.rerank_pool_size,
            rerank_keep_top: summary.rerank_keep_top,
        };

        let llm = if summary.llm_cases > 0 {
            Some(LlmSection {
                cases: summary.llm_cases,
                answered: summary.llm_answered,
                precision: summary.llm_precision,
            })
        } else {
            None
        };

        let performance = PerformanceSection {
            openai_base_url: summary.perf.openai_base_url.clone(),
            ingestion_ms: summary.perf.ingestion_ms,
            namespace_seed_ms: summary.perf.namespace_seed_ms,
            evaluation_stages_ms: summary.perf.evaluation_stage_ms.clone(),
            stage_latency: summary.perf.stage_latency.clone(),
            namespace_reused: summary.namespace_reused,
            ingestion_reused: summary.ingestion_reused,
            embeddings_reused: summary.ingestion_embeddings_reused,
            ingestion_cache_path: summary.ingestion_cache_path.clone(),
            corpus_paragraphs: summary.corpus_paragraphs,
            positive_paragraphs_reused: summary.positive_paragraphs_reused,
            negative_paragraphs_reused: summary.negative_paragraphs_reused,
        };

        let misses = summary
            .cases
            .iter()
            .filter(|case| !case.matched && !case.is_impossible)
            .take(sample)
            .map(MissEntry::from_case)
            .collect();

        let llm_cases = if llm.is_some() {
            summary
                .cases
                .iter()
                .filter(|case| case.is_impossible)
                .take(sample)
                .map(LlmCaseEntry::from_case)
                .collect()
        } else {
            Vec::new()
        };

        Self {
            overview,
            dataset,
            slice,
            retrieval,
            llm,
            performance,
            misses,
            llm_cases,
            detailed_report: summary.detailed_report,
        }
    }
}

impl MissEntry {
    fn from_case(case: &CaseSummary) -> Self {
        Self {
            question_id: case.question_id.clone(),
            paragraph_title: case.paragraph_title.clone(),
            expected_source: case.expected_source.clone(),
            entity_match: case.entity_match,
            chunk_text_match: case.chunk_text_match,
            chunk_id_match: case.chunk_id_match,
            retrieved: case
                .retrieved
                .iter()
                .take(3)
                .map(RetrievedSnippet::from_summary)
                .collect(),
        }
    }
}

impl LlmCaseEntry {
    fn from_case(case: &CaseSummary) -> Self {
        Self {
            question_id: case.question_id.clone(),
            answered: case.matched,
            match_rank: case.match_rank,
            retrieved: case
                .retrieved
                .iter()
                .take(3)
                .map(RetrievedSnippet::from_summary)
                .collect(),
        }
    }
}

impl RetrievedSnippet {
    fn from_summary(entry: &crate::eval::RetrievedSummary) -> Self {
        Self {
            rank: entry.rank,
            source_id: entry.source_id.clone(),
            entity_name: entry.entity_name.clone(),
            matched: entry.matched,
        }
    }
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
    let report = EvaluationReport::from_summary(summary, sample);

    let json_path = dataset_dir.join(format!("{stem}.json"));
    let json_blob = serde_json::to_string_pretty(&report).context("serialising JSON report")?;
    fs::write(&json_path, &json_blob)
        .with_context(|| format!("writing JSON report to {}", json_path.display()))?;

    let md_path = dataset_dir.join(format!("{stem}.md"));
    let markdown = render_markdown(&report);
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

fn render_markdown(report: &EvaluationReport) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "# Retrieval Evaluation (k={})\\n\\n",
        report.retrieval.k
    ));

    md.push_str("## Overview\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    md.push_str(&format!(
        "| Generated | {} |\\n",
        report.overview.generated_at
    ));
    md.push_str(&format!(
        "| Run Label | {} |\\n",
        report
            .overview
            .run_label
            .as_deref()
            .filter(|label| !label.is_empty())
            .unwrap_or("-")
    ));
    md.push_str(&format!(
        "| Total Cases | {} |\\n",
        report.overview.total_cases
    ));
    md.push_str(&format!(
        "| Filtered Questions | {} |\\n",
        report.overview.filtered_questions
    ));

    md.push_str("\\n## Dataset & Slice\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    md.push_str(&format!(
        "| Dataset | {} (`{}`) |\\n",
        report.dataset.label, report.dataset.id
    ));
    md.push_str(&format!(
        "| Dataset Source | {} |\\n",
        report.dataset.source
    ));
    md.push_str(&format!(
        "| Includes Unanswerable | {} |\\n",
        bool_badge(report.dataset.includes_unanswerable)
    ));
    md.push_str(&format!(
        "| Require Verified Chunks | {} |\\n",
        bool_badge(report.dataset.require_verified_chunks)
    ));
    let embedding_label = if let Some(model) = report.dataset.embedding_model.as_ref() {
        format!("{} ({model})", report.dataset.embedding_backend)
    } else {
        report.dataset.embedding_backend.clone()
    };
    md.push_str(&format!("| Embedding | {} |\\n", embedding_label));
    md.push_str(&format!(
        "| Embedding Dim | {} |\\n",
        report.dataset.embedding_dimension
    ));
    md.push_str(&format!("| Slice ID | `{}` |\\n", report.slice.id));
    md.push_str(&format!("| Slice Seed | {} |\\n", report.slice.seed));
    md.push_str(&format!(
        "| Slice Window (offset/length) | {}/{} |\\n",
        report.slice.window_offset, report.slice.window_length
    ));
    md.push_str(&format!(
        "| Slice Questions (window/ledger) | {}/{} |\\n",
        report.slice.slice_cases, report.slice.ledger_total_cases
    ));
    md.push_str(&format!(
        "| Slice Positives / Negatives | {}/{} |\\n",
        report.slice.positives, report.slice.negatives
    ));
    md.push_str(&format!(
        "| Slice Paragraphs | {} |\\n",
        report.slice.total_paragraphs
    ));
    md.push_str(&format!(
        "| Negative Multiplier | {:.2} |\\n",
        report.slice.negative_multiplier
    ));

    md.push_str("\\n## Retrieval Metrics\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    md.push_str(&format!("| Cases | {} |\\n", report.retrieval.cases));
    md.push_str(&format!(
        "| Correct@{} | {}/{} |\\n",
        report.retrieval.k, report.retrieval.correct, report.retrieval.cases
    ));
    md.push_str(&format!(
        "| Precision@{} | {:.3} |\\n",
        report.retrieval.k, report.retrieval.precision
    ));
    md.push_str(&format!(
        "| Precision@1/2/3 | {:.3} / {:.3} / {:.3} |\\n",
        report.retrieval.precision_at_1,
        report.retrieval.precision_at_2,
        report.retrieval.precision_at_3
    ));
    md.push_str(&format!(
        "| MRR | {:.3} |\\n",
        report.retrieval.mrr
    ));
    md.push_str(&format!(
        "| NDCG | {:.3} |\\n",
        report.retrieval.average_ndcg
    ));
    md.push_str(&format!(
        "| Latency Avg / P50 / P95 (ms) | {:.1} / {} / {} |\\n",
        report.retrieval.latency.avg, report.retrieval.latency.p50, report.retrieval.latency.p95
    ));
    md.push_str(&format!(
        "| Strategy | `{}` |\\n",
        report.retrieval.strategy
    ));
    md.push_str(&format!(
        "| Concurrency | {} |\\n",
        report.retrieval.concurrency
    ));
    if report.retrieval.rerank_enabled {
        let pool = report
            .retrieval
            .rerank_pool_size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "?".into());
        md.push_str(&format!(
            "| Rerank | enabled (pool {pool}, keep top {}) |\\n",
            report.retrieval.rerank_keep_top
        ));
    } else {
        md.push_str("| Rerank | disabled |\\n");
    }

    if let Some(llm) = &report.llm {
        md.push_str("\\n## LLM Mode Metrics\\n\\n");
        md.push_str("| Metric | Value |\\n| --- | --- |\\n");
        md.push_str(&format!("| Cases | {} |\\n", llm.cases));
        md.push_str(&format!("| Answered | {} |\\n", llm.answered));
        md.push_str(&format!("| Precision | {:.3} |\\n", llm.precision));
    }

    md.push_str("\\n## Performance\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    md.push_str(&format!(
        "| OpenAI Base URL | {} |\\n",
        report.performance.openai_base_url
    ));
    md.push_str(&format!(
        "| Ingestion Duration | {} ms |\\n",
        report.performance.ingestion_ms
    ));
    if let Some(seed) = report.performance.namespace_seed_ms {
        md.push_str(&format!("| Namespace Seed | {} ms |\\n", seed));
    }
    md.push_str(&format!(
        "| Namespace State | {} |\\n",
        if report.performance.namespace_reused {
            "reused"
        } else {
            "seeded"
        }
    ));
    md.push_str(&format!(
        "| Corpus Paragraphs | {} |\\n",
        report.performance.corpus_paragraphs
    ));
    if report.detailed_report {
        md.push_str(&format!(
            "| Ingestion Cache | `{}` |\\n",
            report.performance.ingestion_cache_path
        ));
        md.push_str(&format!(
            "| Ingestion Reused | {} |\\n",
            bool_badge(report.performance.ingestion_reused)
        ));
        md.push_str(&format!(
            "| Embeddings Reused | {} |\\n",
            bool_badge(report.performance.embeddings_reused)
        ));
    }
    md.push_str(&format!(
        "| Positives Cached | {} |\\n",
        report.performance.positive_paragraphs_reused
    ));
    md.push_str(&format!(
        "| Negatives Cached | {} |\\n",
        report.performance.negative_paragraphs_reused
    ));

    md.push_str("\\n## Retrieval Stage Timings\\n\\n");
    md.push_str("| Stage | Avg (ms) | P50 (ms) | P95 (ms) |\\n| --- | --- | --- | --- |\\n");
    write_stage_row(&mut md, "Embed", &report.performance.stage_latency.embed);
    write_stage_row(
        &mut md,
        "Collect Candidates",
        &report.performance.stage_latency.collect_candidates,
    );
    write_stage_row(
        &mut md,
        "Graph Expansion",
        &report.performance.stage_latency.graph_expansion,
    );
    write_stage_row(
        &mut md,
        "Chunk Attach",
        &report.performance.stage_latency.chunk_attach,
    );
    write_stage_row(&mut md, "Rerank", &report.performance.stage_latency.rerank);
    write_stage_row(
        &mut md,
        "Assemble",
        &report.performance.stage_latency.assemble,
    );

    if report.misses.is_empty() {
        md.push_str("\\n_All evaluated retrieval queries matched within the top-k window._\\n");
        if report.detailed_report {
            md.push_str(
                "\\nSuccess measures were captured for each query (entity, chunk text, chunk ID).\\n",
            );
        }
    } else {
        md.push_str("\\n## Missed Retrieval Queries (sample)\\n\\n");
        if report.detailed_report {
            md.push_str(
                "| Question ID | Paragraph | Expected Source | Entity Match | Chunk Text | Chunk ID | Top Retrieved |\\n",
            );
            md.push_str("| --- | --- | --- | --- | --- | --- | --- |\\n");
        } else {
            md.push_str("| Question ID | Paragraph | Expected Source | Top Retrieved |\\n");
            md.push_str("| --- | --- | --- | --- |\\n");
        }
        for case in &report.misses {
            let retrieved = render_retrieved(&case.retrieved);
            if report.detailed_report {
                md.push_str(&format!(
                    "| `{}` | {} | `{}` | {} | {} | {} | {} |\\n",
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
                    "| `{}` | {} | `{}` | {} |\\n",
                    case.question_id, case.paragraph_title, case.expected_source, retrieved
                ));
            }
        }
    }

    if report.llm.is_some() {
        md.push_str("\\n## LLM-Only Cases (sample)\\n\\n");
        if report.llm_cases.is_empty() {
            md.push_str("All LLM-only cases matched within the evaluation window.\\n");
        } else {
            md.push_str("| Question ID | Answered | Match Rank | Top Retrieved |\\n");
            md.push_str("| --- | --- | --- | --- |\\n");
            for case in &report.llm_cases {
                let retrieved = render_retrieved(&case.retrieved);
                let rank = case
                    .match_rank
                    .map(|rank| rank.to_string())
                    .unwrap_or_else(|| "-".into());
                md.push_str(&format!(
                    "| `{}` | {} | {} | {} |\\n",
                    case.question_id,
                    bool_badge(case.answered),
                    rank,
                    retrieved
                ));
            }
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

fn render_retrieved(entries: &[RetrievedSnippet]) -> String {
    if entries.is_empty() {
        "-".to_string()
    } else {
        entries
            .iter()
            .map(|entry| format!("{} (rank {})", entry.source_id, entry.rank))
            .take(3)
            .collect::<Vec<_>>()
            .join("<br>")
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
    #[serde(default)]
    mrr: f64,
    #[serde(default)]
    average_ndcg: f64,
    #[serde(default)]
    retrieval_cases: usize,
    #[serde(default)]
    retrieval_precision: f64,
    #[serde(default)]
    llm_cases: usize,
    #[serde(default)]
    llm_precision: f64,
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
        match serde_json::from_slice(&contents) {
            Ok(entries) => entries,
            Err(err) => {
                let timestamp = Utc::now().format("%Y%m%dT%H%M%S");
                let backup_path =
                    report_dir.join(format!("evaluations.json.corrupted.{}", timestamp));
                warn!(
                    path = %path.display(),
                    backup = %backup_path.display(),
                    error = %err,
                    "Evaluation history file is corrupted; backing up and starting fresh"
                );
                if let Err(e) = fs::rename(&path, &backup_path) {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to backup corrupted evaluation history"
                    );
                }
                Vec::new()
            }
        }
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
        mrr: summary.mrr,
        average_ndcg: summary.average_ndcg,
        retrieval_cases: summary.retrieval_cases,
        retrieval_precision: summary.retrieval_precision,
        llm_cases: summary.llm_cases,
        llm_precision: summary.llm_precision,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{
        EvaluationStageTimings, PerformanceTimings, RetrievedSummary, StageLatencyBreakdown,
    };
    use chrono::Utc;

    fn latency(ms: f64) -> LatencyStats {
        LatencyStats {
            avg: ms,
            p50: ms as u128,
            p95: ms as u128,
        }
    }

    fn sample_stage_latency() -> StageLatencyBreakdown {
        StageLatencyBreakdown {
            embed: latency(9.0),
            collect_candidates: latency(10.0),
            graph_expansion: latency(11.0),
            chunk_attach: latency(12.0),
            rerank: latency(13.0),
            assemble: latency(14.0),
        }
    }

    fn sample_eval_stage() -> EvaluationStageTimings {
        EvaluationStageTimings {
            prepare_slice_ms: 1,
            prepare_db_ms: 2,
            prepare_corpus_ms: 3,
            prepare_namespace_ms: 4,
            run_queries_ms: 5,
            summarize_ms: 6,
            finalize_ms: 7,
        }
    }

    fn sample_case(is_impossible: bool, matched: bool) -> CaseSummary {
        CaseSummary {
            question_id: if is_impossible {
                "llm-q".into()
            } else {
                "retrieval-q".into()
            },
            question: "Who is the hero?".into(),
            paragraph_id: "p1".into(),
            paragraph_title: "Hero".into(),
            expected_source: "src1".into(),
            answers: vec!["answer".into()],
            matched,
            entity_match: matched,
            chunk_text_match: matched,
            chunk_id_match: matched,
            is_impossible,
            has_verified_chunks: !is_impossible,
            match_rank: if matched { Some(1) } else { None },
            latency_ms: 42,
            retrieved: vec![RetrievedSummary {
                rank: 1,
                entity_id: "entity1".into(),
                source_id: "src1".into(),
                entity_name: "Entity".into(),
                score: 1.0,
                matched,
                entity_description: None,
                entity_category: None,
                chunk_text_match: Some(matched),
                chunk_id_match: Some(matched),
            }],
        }
    }

    fn sample_summary(include_llm: bool) -> EvaluationSummary {
        let mut cases = vec![sample_case(false, true)];
        if include_llm {
            cases.push(sample_case(true, false));
        }
        EvaluationSummary {
            generated_at: Utc::now(),
            k: 5,
            limit: Some(10),
            run_label: Some("test".into()),
            total_cases: cases.len(),
            correct: 1,
            precision: 1.0,
            correct_at_1: 1,
            correct_at_2: 1,
            correct_at_3: 1,
            precision_at_1: 1.0,
            precision_at_2: 1.0,
            precision_at_3: 1.0,
            duration_ms: 100,
            dataset_id: "ds".into(),
            dataset_label: "Dataset".into(),
            dataset_includes_unanswerable: include_llm,
            dataset_source: "dev".into(),
            includes_impossible_cases: include_llm,
            require_verified_chunks: !include_llm,
            filtered_questions: 0,
            retrieval_cases: 1,
            retrieval_correct: 1,
            retrieval_precision: 1.0,
            llm_cases: if include_llm { 1 } else { 0 },
            llm_answered: 0,
            llm_precision: 0.0,
            slice_id: "slice".into(),
            slice_seed: 1,
            slice_total_cases: cases.len(),
            slice_window_offset: 0,
            slice_window_length: cases.len(),
            slice_cases: cases.len(),
            slice_positive_paragraphs: 1,
            slice_negative_paragraphs: 0,
            slice_total_paragraphs: 1,
            slice_negative_multiplier: 1.0,
            namespace_reused: true,
            corpus_paragraphs: 1,
            ingestion_cache_path: "/cache".into(),
            ingestion_reused: true,
            ingestion_embeddings_reused: true,
            ingestion_fingerprint: "fp".into(),
            positive_paragraphs_reused: 1,
            negative_paragraphs_reused: 0,
            latency_ms: latency(10.0),
            perf: PerformanceTimings {
                openai_base_url: "https://example.com".into(),
                ingestion_ms: 100,
                namespace_seed_ms: Some(50),
                evaluation_stage_ms: sample_eval_stage(),
                stage_latency: sample_stage_latency(),
            },
            embedding_backend: "fastembed".into(),
            embedding_model: Some("model".into()),
            embedding_dimension: 32,
            rerank_enabled: true,
            rerank_pool_size: Some(4),
            rerank_keep_top: 5,
            concurrency: 2,
            detailed_report: true,
            retrieval_strategy: "initial".into(),
            chunk_vector_take: 50,
            chunk_fts_take: 50,
            chunk_token_budget: 10_000,
            chunk_avg_chars_per_token: 4,
            max_chunks_per_entity: 4,
            cases,
        }
    }

    #[test]
    fn markdown_includes_llm_section() {
        let summary = sample_summary(true);
        let report = EvaluationReport::from_summary(&summary, 5);
        let md = render_markdown(&report);
        assert!(md.contains("LLM Mode Metrics"));
        assert!(md.contains("LLM-Only Cases (sample)"));
    }

    #[test]
    fn markdown_hides_llm_section_when_not_present() {
        let summary = sample_summary(false);
        let report = EvaluationReport::from_summary(&summary, 5);
        let md = render_markdown(&report);
        assert!(!md.contains("LLM Mode Metrics"));
        assert!(!md.contains("LLM-Only Cases"));
    }
}
