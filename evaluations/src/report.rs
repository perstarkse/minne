use std::{
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{
    CaseSummary, EvaluationStageTimings, EvaluationSummary, LatencyStats, RetrievalContextStats,
    StageLatencyBreakdown, format_timestamp,
};

#[derive(Debug)]
pub struct ReportPaths {
    pub json: PathBuf,
    pub markdown: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationReport {
    pub overview: OverviewSection,
    pub dataset: DatasetSection,
    pub slice: SliceSection,
    pub retrieval: RetrievalSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmSection>,
    pub performance: PerformanceSection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub misses: Vec<MissEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub llm_cases: Vec<LlmCaseEntry>,
    #[serde(default)]
    pub detailed_report: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverviewSection {
    pub generated_at: String,
    pub run_label: Option<String>,
    pub total_cases: usize,
    pub filtered_questions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
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
    pub resolve_entities: bool,
    pub rerank_enabled: bool,
    pub rerank_pool_size: Option<usize>,
    pub rerank_keep_top: usize,
    pub chunk_result_cap: usize,
    #[serde(default = "default_chunk_rrf_k")]
    pub chunk_rrf_k: f32,
    #[serde(default = "default_chunk_rrf_weight")]
    pub chunk_rrf_vector_weight: f32,
    #[serde(default = "default_chunk_rrf_weight")]
    pub chunk_rrf_fts_weight: f32,
    #[serde(default = "default_chunk_rrf_use")]
    pub chunk_rrf_use_vector: bool,
    #[serde(default = "default_chunk_rrf_use")]
    pub chunk_rrf_use_fts: bool,
    #[serde(default)]
    pub chunk_vector_take: usize,
    #[serde(default)]
    pub chunk_fts_take: usize,
    pub ingest_chunk_min_tokens: usize,
    pub ingest_chunk_max_tokens: usize,
    pub ingest_chunk_overlap_tokens: usize,
    pub ingest_chunks_only: bool,
    pub retrieved_context: RetrievalContextStats,
}

const fn default_chunk_rrf_k() -> f32 {
    60.0
}

const fn default_chunk_rrf_weight() -> f32 {
    1.0
}

const fn default_chunk_rrf_use() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSection {
    pub cases: usize,
    pub answered: usize,
    pub precision: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissEntry {
    pub question_id: String,
    pub paragraph_title: String,
    pub expected_source: String,
    pub entity_match: bool,
    pub chunk_text_match: bool,
    pub chunk_id_match: bool,
    pub retrieved: Vec<RetrievedSnippet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCaseEntry {
    pub question_id: String,
    pub answered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_rank: Option<usize>,
    pub retrieved: Vec<RetrievedSnippet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedSnippet {
    pub rank: usize,
    pub source_id: String,
    pub entity_name: String,
    pub matched: bool,
}

#[derive(Debug)]
pub struct ReportOutcome {
    pub record: EvaluationReport,
    pub paths: ReportPaths,
    pub history_path: PathBuf,
}

impl EvaluationReport {
    #[allow(clippy::too_many_lines)]
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
            resolve_entities: summary.resolve_entities,
            rerank_enabled: summary.rerank_enabled,
            rerank_pool_size: summary.rerank_pool_size,
            rerank_keep_top: summary.rerank_keep_top,
            chunk_result_cap: summary.chunk_result_cap,
            chunk_rrf_k: summary.chunk_rrf_k,
            chunk_rrf_vector_weight: summary.chunk_rrf_vector_weight,
            chunk_rrf_fts_weight: summary.chunk_rrf_fts_weight,
            chunk_rrf_use_vector: summary.chunk_rrf_use_vector,
            chunk_rrf_use_fts: summary.chunk_rrf_use_fts,
            chunk_vector_take: summary.chunk_vector_take,
            chunk_fts_take: summary.chunk_fts_take,
            ingest_chunk_min_tokens: summary.ingest_chunk_min_tokens,
            ingest_chunk_max_tokens: summary.ingest_chunk_max_tokens,
            ingest_chunk_overlap_tokens: summary.ingest_chunk_overlap_tokens,
            ingest_chunks_only: summary.ingest_chunks_only,
            retrieved_context: summary.retrieved_context.clone(),
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

        let (misses, llm_cases) = if summary.detailed_report {
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

            (misses, llm_cases)
        } else {
            (Vec::new(), Vec::new())
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
    fn from_summary(entry: &crate::types::RetrievedSummary) -> Self {
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
) -> Result<ReportOutcome> {
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

    let history_path = record_history(&report, &dataset_dir)?;

    Ok(ReportOutcome {
        record: report,
        paths: ReportPaths {
            json: json_path,
            markdown: md_path,
        },
        history_path,
    })
}

#[allow(
    clippy::too_many_lines,
    clippy::write_with_newline,
    clippy::unwrap_used
)]
fn render_markdown(report: &EvaluationReport) -> String {
    let mut md = String::new();

    write!(
        md,
        "# Retrieval Evaluation (k={})\\n\\n",
        report.retrieval.k
    )
    .unwrap();

    md.push_str("## Overview\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    write!(md, "| Generated | {} |\\n", report.overview.generated_at).unwrap();
    write!(
        md,
        "| Run Label | {} |\\n",
        report
            .overview
            .run_label
            .as_deref()
            .filter(|label| !label.is_empty())
            .unwrap_or("-")
    )
    .unwrap();
    write!(md, "| Total Cases | {} |\\n", report.overview.total_cases).unwrap();
    write!(
        md,
        "| Filtered Questions | {} |\\n",
        report.overview.filtered_questions
    )
    .unwrap();

    md.push_str("\\n## Dataset & Slice\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    write!(
        md,
        "| Dataset | {} (`{}`) |\\n",
        report.dataset.label, report.dataset.id
    )
    .unwrap();
    write!(md, "| Dataset Source | {} |\\n", report.dataset.source).unwrap();
    write!(
        md,
        "| Includes Unanswerable | {} |\\n",
        bool_badge(report.dataset.includes_unanswerable)
    )
    .unwrap();
    write!(
        md,
        "| Require Verified Chunks | {} |\\n",
        bool_badge(report.dataset.require_verified_chunks)
    )
    .unwrap();
    let embedding_label = if let Some(model) = report.dataset.embedding_model.as_ref() {
        format!("{} ({model})", report.dataset.embedding_backend)
    } else {
        report.dataset.embedding_backend.clone()
    };
    write!(md, "| Embedding | {embedding_label} |\\n").unwrap();
    write!(
        md,
        "| Embedding Dim | {} |\\n",
        report.dataset.embedding_dimension
    )
    .unwrap();
    write!(md, "| Slice ID | `{}` |\\n", report.slice.id).unwrap();
    write!(md, "| Slice Seed | {} |\\n", report.slice.seed).unwrap();
    write!(
        md,
        "| Slice Window (offset/length) | {}/{} |\\n",
        report.slice.window_offset, report.slice.window_length
    )
    .unwrap();
    write!(
        md,
        "| Slice Questions (window/ledger) | {}/{} |\\n",
        report.slice.slice_cases, report.slice.ledger_total_cases
    )
    .unwrap();
    write!(
        md,
        "| Slice Positives / Negatives | {}/{} |\\n",
        report.slice.positives, report.slice.negatives
    )
    .unwrap();
    write!(
        md,
        "| Slice Paragraphs | {} |\\n",
        report.slice.total_paragraphs
    )
    .unwrap();
    write!(
        md,
        "| Negative Multiplier | {:.2} |\\n",
        report.slice.negative_multiplier
    )
    .unwrap();

    md.push_str("\\n## Retrieval Metrics\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    write!(md, "| Cases | {} |\\n", report.retrieval.cases).unwrap();
    write!(
        md,
        "| Correct@{} | {}/{} |\\n",
        report.retrieval.k, report.retrieval.correct, report.retrieval.cases
    )
    .unwrap();
    write!(
        md,
        "| Precision@{} | {:.3} |\\n",
        report.retrieval.k, report.retrieval.precision
    )
    .unwrap();
    write!(
        md,
        "| Precision@1/2/3 | {:.3} / {:.3} / {:.3} |\\n",
        report.retrieval.precision_at_1,
        report.retrieval.precision_at_2,
        report.retrieval.precision_at_3
    )
    .unwrap();
    write!(md, "| MRR | {:.3} |\\n", report.retrieval.mrr).unwrap();
    write!(md, "| NDCG | {:.3} |\\n", report.retrieval.average_ndcg).unwrap();
    write!(
        md,
        "| Latency Avg / P50 / P95 (ms) | {:.1} / {} / {} |\\n",
        report.retrieval.latency.avg, report.retrieval.latency.p50, report.retrieval.latency.p95
    )
    .unwrap();
    write!(
        md,
        "| Resolve entities | {} |\\n",
        bool_badge(report.retrieval.resolve_entities)
    )
    .unwrap();
    write!(md, "| Concurrency | {} |\\n", report.retrieval.concurrency).unwrap();
    if report.retrieval.rerank_enabled {
        let pool = report
            .retrieval
            .rerank_pool_size
            .map_or_else(|| "?".into(), |size| size.to_string());
        write!(
            md,
            "| Rerank | enabled (pool {pool}, keep top {}) |\\n",
            report.retrieval.rerank_keep_top
        )
        .unwrap();
    } else {
        md.push_str("| Rerank | disabled |\\n");
    }
    write!(
        md,
        "| Chunk result cap | {} |\\n",
        report.retrieval.chunk_result_cap
    )
    .unwrap();

    md.push_str("\\n## Retrieved Context Volume\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    write!(
        md,
        "| Tokenizer | {} |\\n",
        report.retrieval.retrieved_context.tokenizer
    )
    .unwrap();
    write!(
        md,
        "| Queries measured | {} |\\n",
        report.retrieval.retrieved_context.queries
    )
    .unwrap();
    write!(
        md,
        "| Total chunks returned | {} |\\n",
        report.retrieval.retrieved_context.total_chunks
    )
    .unwrap();
    write!(
        md,
        "| Total characters | {} |\\n",
        report.retrieval.retrieved_context.total_chars
    )
    .unwrap();
    write!(
        md,
        "| Total tokens | {} |\\n",
        report.retrieval.retrieved_context.total_tokens
    )
    .unwrap();
    write!(
        md,
        "| Avg chunks / query | {:.1} |\\n",
        report.retrieval.retrieved_context.avg_chunks_per_query
    )
    .unwrap();
    write!(
        md,
        "| Avg tokens / query | {:.1} |\\n",
        report.retrieval.retrieved_context.avg_tokens_per_query
    )
    .unwrap();
    write!(
        md,
        "| P50 / P95 / max tokens / query | {} / {} / {} |\\n",
        report.retrieval.retrieved_context.p50_tokens_per_query,
        report.retrieval.retrieved_context.p95_tokens_per_query,
        report.retrieval.retrieved_context.max_tokens_per_query
    )
    .unwrap();

    if let Some(llm) = &report.llm {
        md.push_str("\\n## LLM Mode Metrics\\n\\n");
        md.push_str("| Metric | Value |\\n| --- | --- |\\n");
        write!(md, "| Cases | {} |\\n", llm.cases).unwrap();
        write!(md, "| Answered | {} |\\n", llm.answered).unwrap();
        write!(md, "| Precision | {:.3} |\\n", llm.precision).unwrap();
    }

    md.push_str("\\n## Performance\\n\\n");
    md.push_str("| Metric | Value |\\n| --- | --- |\\n");
    write!(
        md,
        "| OpenAI Base URL | {} |\\n",
        report.performance.openai_base_url
    )
    .unwrap();
    write!(
        md,
        "| Ingestion Duration | {} ms |\\n",
        report.performance.ingestion_ms
    )
    .unwrap();
    if let Some(seed) = report.performance.namespace_seed_ms {
        write!(md, "| Namespace Seed | {seed} ms |\\n").unwrap();
    }
    write!(
        md,
        "| Namespace State | {} |\\n",
        if report.performance.namespace_reused {
            "reused"
        } else {
            "seeded"
        }
    )
    .unwrap();
    write!(
        md,
        "| Corpus Paragraphs | {} |\\n",
        report.performance.corpus_paragraphs
    )
    .unwrap();
    if report.detailed_report {
        write!(
            md,
            "| Ingestion Cache | `{}` |\\n",
            report.performance.ingestion_cache_path
        )
        .unwrap();
        write!(
            md,
            "| Ingestion Reused | {} |\\n",
            bool_badge(report.performance.ingestion_reused)
        )
        .unwrap();
        write!(
            md,
            "| Embeddings Reused | {} |\\n",
            bool_badge(report.performance.embeddings_reused)
        )
        .unwrap();
    }
    write!(
        md,
        "| Positives Cached | {} |\\n",
        report.performance.positive_paragraphs_reused
    )
    .unwrap();
    write!(
        md,
        "| Negatives Cached | {} |\\n",
        report.performance.negative_paragraphs_reused
    )
    .unwrap();

    md.push_str("\\n## Retrieval Stage Timings\\n\\n");
    md.push_str("| Stage | Avg (ms) | P50 (ms) | P95 (ms) |\\n| --- | --- | --- | --- |\\n");
    for stage in &report.performance.stage_latency.stages {
        write_stage_row(&mut md, &prettify_stage(&stage.stage), &stage.stats);
    }

    if report.misses.is_empty() {
        if report.detailed_report {
            md.push_str(
                "\\n_All evaluated retrieval queries matched within the top-k window._\\n\
                \\nSuccess measures were captured for each query (entity, chunk text, chunk ID).\\n",
            );
        } else {
            md.push_str(
                "\\n_Misses omitted. Re-run with `--detailed-report` to see sampled failures._\\n",
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
                write!(
                    md,
                    "| `{}` | {} | `{}` | {} | {} | {} | {} |\\n",
                    case.question_id,
                    case.paragraph_title,
                    case.expected_source,
                    bool_badge(case.entity_match),
                    bool_badge(case.chunk_text_match),
                    bool_badge(case.chunk_id_match),
                    retrieved
                )
                .unwrap();
            } else {
                write!(
                    md,
                    "| `{}` | {} | `{}` | {} |\\n",
                    case.question_id, case.paragraph_title, case.expected_source, retrieved
                )
                .unwrap();
            }
        }
    }

    if report.llm.is_some() {
        md.push_str("\\n## LLM-Only Cases (sample)\\n\\n");
        if report.llm_cases.is_empty() {
            if report.detailed_report {
                md.push_str("All LLM-only cases matched within the evaluation window.\\n");
            } else {
                md.push_str(
                    "LLM-only cases omitted. Re-run with `--detailed-report` to see samples.\\n",
                );
            }
        } else {
            md.push_str("| Question ID | Answered | Match Rank | Top Retrieved |\\n");
            md.push_str("| --- | --- | --- | --- |\\n");
            for case in &report.llm_cases {
                let retrieved = render_retrieved(&case.retrieved);
                let rank = case
                    .match_rank
                    .map_or_else(|| "-".into(), |rank| rank.to_string());
                write!(
                    md,
                    "| `{}` | {} | {} | {} |\\n",
                    case.question_id,
                    bool_badge(case.answered),
                    rank,
                    retrieved
                )
                .unwrap();
            }
        }
    }

    md
}
#[allow(clippy::write_with_newline, clippy::unwrap_used)]
fn write_stage_row(buf: &mut String, label: &str, stats: &LatencyStats) {
    writeln!(
        buf,
        "| {} | {:.1} | {} | {} |",
        label, stats.avg, stats.p50, stats.p95
    )
    .unwrap();
}

/// Turn a stable stage label (e.g. `resolve_entities`) into a display title (`Resolve Entities`).
fn prettify_stage(label: &str) -> String {
    label
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn bool_badge(value: bool) -> &'static str {
    if value { "✅" } else { "⚪" }
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

fn load_history(path: &Path) -> Result<Vec<EvaluationReport>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents =
        fs::read(path).with_context(|| format!("reading evaluation log {}", path.display()))?;

    serde_json::from_slice(&contents).with_context(|| {
        format!(
            "parsing evaluation history at {}; delete the file and re-run if upgrading from an older format",
            path.display()
        )
    })
}

fn record_history(report: &EvaluationReport, report_dir: &Path) -> Result<PathBuf> {
    let path = report_dir.join("evaluations.json");
    let mut entries = load_history(&path)?;
    entries.push(report.clone());

    let blob = serde_json::to_vec_pretty(&entries).context("serialising evaluation log")?;
    fs::write(&path, blob).with_context(|| format!("writing evaluation log {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        EvaluationStageTimings, PerformanceTimings, RetrievedContextStats, RetrievedSummary,
        StageLatency, StageLatencyBreakdown,
    };
    use chrono::Utc;
    use tempfile::tempdir;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn latency(ms: f64) -> LatencyStats {
        LatencyStats {
            avg: ms,
            p50: ms as u128,
            p95: ms as u128,
        }
    }

    fn sample_stage_latency() -> StageLatencyBreakdown {
        StageLatencyBreakdown {
            stages: vec![
                StageLatency {
                    stage: "embed".to_string(),
                    stats: latency(9.0),
                },
                StageLatency {
                    stage: "search".to_string(),
                    stats: latency(10.0),
                },
                StageLatency {
                    stage: "rerank".to_string(),
                    stats: latency(13.0),
                },
                StageLatency {
                    stage: "resolve_entities".to_string(),
                    stats: latency(11.0),
                },
                StageLatency {
                    stage: "assemble".to_string(),
                    stats: latency(14.0),
                },
            ],
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
            ndcg: None,
            reciprocal_rank: None,
            is_impossible,
            has_verified_chunks: !is_impossible,
            match_rank: if matched { Some(1) } else { None },
            latency_ms: 42,
            retrieved_context: RetrievedContextStats::default(),
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
            average_ndcg: 0.0,
            mrr: 0.0,
            llm_cases: usize::from(include_llm),
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
            resolve_entities: false,
            chunk_result_cap: 5,
            chunk_rrf_k: 60.0,
            chunk_rrf_vector_weight: 1.0,
            chunk_rrf_fts_weight: 1.0,
            chunk_rrf_use_vector: true,
            chunk_rrf_use_fts: true,
            ingest_chunk_min_tokens: 256,
            ingest_chunk_max_tokens: 512,
            ingest_chunk_overlap_tokens: 50,
            ingest_chunks_only: false,
            chunk_vector_take: 50,
            chunk_fts_take: 50,
            max_chunks_per_entity: 4,
            retrieved_context: crate::context_stats::aggregate_context_stats(&[
                RetrievedContextStats {
                    chunk_count: 1,
                    char_count: 10,
                    token_count: 3,
                },
            ]),
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

    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    #[test]
    fn evaluations_history_captures_resolve_entities_and_concurrency() {
        let tmp = tempdir().unwrap();
        let summary = sample_summary(false);

        let outcome = write_reports(&summary, tmp.path(), 5).expect("writing consolidated reports");
        let contents =
            std::fs::read_to_string(&outcome.history_path).expect("reading evaluations history");
        let entries: Vec<EvaluationReport> =
            serde_json::from_str(&contents).expect("parsing evaluations history");
        assert_eq!(entries.len(), 1);
        let stored = &entries[0];
        assert_eq!(stored.retrieval.concurrency, summary.concurrency);
        assert_eq!(stored.retrieval.resolve_entities, summary.resolve_entities);
        assert_eq!(
            stored.performance.evaluation_stages_ms.run_queries_ms,
            summary.perf.evaluation_stage_ms.run_queries_ms
        );
    }
}
