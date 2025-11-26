use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use retrieval_pipeline::RetrievalStrategy;

use crate::datasets::DatasetKind;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf()
}

fn default_report_dir() -> PathBuf {
    workspace_root().join("eval/reports")
}

fn default_cache_dir() -> PathBuf {
    workspace_root().join("eval/cache")
}

fn default_ingestion_cache_dir() -> PathBuf {
    workspace_root().join("eval/cache/ingested")
}

pub const DEFAULT_SLICE_SEED: u64 = 0x5eed_2025;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingBackend {
    Hashed,
    FastEmbed,
}

impl Default for EmbeddingBackend {
    fn default() -> Self {
        Self::FastEmbed
    }
}

impl std::str::FromStr for EmbeddingBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "hashed" => Ok(Self::Hashed),
            "fastembed" | "fast-embed" | "fast" => Ok(Self::FastEmbed),
            other => Err(anyhow!(
                "unknown embedding backend '{other}'. Expected 'hashed' or 'fastembed'."
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetrievalSettings {
    pub chunk_min_chars: usize,
    pub chunk_max_chars: usize,
    pub chunk_vector_take: Option<usize>,
    pub chunk_fts_take: Option<usize>,
    pub chunk_token_budget: Option<usize>,
    pub chunk_avg_chars_per_token: Option<usize>,
    pub max_chunks_per_entity: Option<usize>,
    pub rerank: bool,
    pub rerank_pool_size: usize,
    pub rerank_keep_top: usize,
    pub require_verified_chunks: bool,
    pub strategy: RetrievalStrategy,
}

impl Default for RetrievalSettings {
    fn default() -> Self {
        Self {
            chunk_min_chars: 500,
            chunk_max_chars: 2_000,
            chunk_vector_take: None,
            chunk_fts_take: None,
            chunk_token_budget: None,
            chunk_avg_chars_per_token: None,
            max_chunks_per_entity: None,
            rerank: true,
            rerank_pool_size: 16,
            rerank_keep_top: 10,
            require_verified_chunks: true,
            strategy: RetrievalStrategy::Initial,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub convert_only: bool,
    pub force_convert: bool,
    pub dataset: DatasetKind,
    pub llm_mode: bool,
    pub corpus_limit: Option<usize>,
    pub raw_dataset_path: PathBuf,
    pub converted_dataset_path: PathBuf,
    pub report_dir: PathBuf,
    pub k: usize,
    pub limit: Option<usize>,
    pub summary_sample: usize,
    pub full_context: bool,
    pub retrieval: RetrievalSettings,
    pub concurrency: usize,
    pub embedding_backend: EmbeddingBackend,
    pub embedding_model: Option<String>,
    pub cache_dir: PathBuf,
    pub ingestion_cache_dir: PathBuf,
    pub ingestion_batch_size: usize,
    pub ingestion_max_retries: usize,
    pub refresh_embeddings_only: bool,
    pub detailed_report: bool,
    pub slice: Option<String>,
    pub reseed_slice: bool,
    pub slice_seed: u64,
    pub slice_grow: Option<usize>,
    pub slice_offset: usize,
    pub slice_reset_ingestion: bool,
    pub negative_multiplier: f32,
    pub label: Option<String>,
    pub chunk_diagnostics_path: Option<PathBuf>,
    pub inspect_question: Option<String>,
    pub inspect_manifest: Option<PathBuf>,
    pub query_model: Option<String>,
    pub perf_log_json: Option<PathBuf>,
    pub perf_log_dir: Option<PathBuf>,
    pub perf_log_console: bool,
    pub db_endpoint: String,
    pub db_username: String,
    pub db_password: String,
    pub db_namespace: Option<String>,
    pub db_database: Option<String>,
    pub inspect_db_state: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        let dataset = DatasetKind::default();
        Self {
            convert_only: false,
            force_convert: false,
            dataset,
            llm_mode: false,
            corpus_limit: None,
            raw_dataset_path: dataset.default_raw_path(),
            converted_dataset_path: dataset.default_converted_path(),
            report_dir: default_report_dir(),
            k: 5,
            limit: Some(200),
            summary_sample: 5,
            full_context: false,
            retrieval: RetrievalSettings::default(),
            concurrency: 4,
            embedding_backend: EmbeddingBackend::FastEmbed,
            embedding_model: None,
            cache_dir: default_cache_dir(),
            ingestion_cache_dir: default_ingestion_cache_dir(),
            ingestion_batch_size: 5,
            ingestion_max_retries: 3,
            refresh_embeddings_only: false,
            detailed_report: false,
            slice: None,
            reseed_slice: false,
            slice_seed: DEFAULT_SLICE_SEED,
            slice_grow: None,
            slice_offset: 0,
            slice_reset_ingestion: false,
            negative_multiplier: crate::slices::DEFAULT_NEGATIVE_MULTIPLIER,
            label: None,
            chunk_diagnostics_path: None,
            inspect_question: None,
            inspect_manifest: None,
            query_model: None,
            inspect_db_state: None,
            perf_log_json: None,
            perf_log_dir: None,
            perf_log_console: false,
            db_endpoint: "ws://127.0.0.1:8000".to_string(),
            db_username: "root_user".to_string(),
            db_password: "root_password".to_string(),
            db_namespace: None,
            db_database: None,
        }
    }
}

impl Config {
    pub fn context_token_limit(&self) -> Option<usize> {
        None
    }
}

#[derive(Debug)]
pub struct ParsedArgs {
    pub config: Config,
    pub show_help: bool,
}

pub fn parse() -> Result<ParsedArgs> {
    let mut config = Config::default();
    let mut show_help = false;
    let mut raw_overridden = false;
    let mut converted_overridden = false;

    let mut args = env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                show_help = true;
                break;
            }
            "--convert-only" => config.convert_only = true,
            "--force" | "--refresh" => config.force_convert = true,
            "--llm-mode" => {
                config.llm_mode = true;
                config.retrieval.require_verified_chunks = false;
            }
            "--dataset" => {
                let value = take_value("--dataset", &mut args)?;
                let parsed = value.parse::<DatasetKind>()?;
                config.dataset = parsed;
                if !raw_overridden {
                    config.raw_dataset_path = parsed.default_raw_path();
                }
                if !converted_overridden {
                    config.converted_dataset_path = parsed.default_converted_path();
                }
            }
            "--slice" => {
                let value = take_value("--slice", &mut args)?;
                config.slice = Some(value);
            }
            "--label" => {
                let value = take_value("--label", &mut args)?;
                config.label = Some(value);
            }
            "--query-model" => {
                let value = take_value("--query-model", &mut args)?;
                if value.trim().is_empty() {
                    return Err(anyhow!("--query-model requires a non-empty model name"));
                }
                config.query_model = Some(value.trim().to_string());
            }
            "--slice-grow" => {
                let value = take_value("--slice-grow", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --slice-grow value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--slice-grow must be greater than zero"));
                }
                config.slice_grow = Some(parsed);
            }
            "--slice-offset" => {
                let value = take_value("--slice-offset", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --slice-offset value '{value}' as usize")
                })?;
                config.slice_offset = parsed;
            }
            "--raw" => {
                let value = take_value("--raw", &mut args)?;
                config.raw_dataset_path = PathBuf::from(value);
                raw_overridden = true;
            }
            "--converted" => {
                let value = take_value("--converted", &mut args)?;
                config.converted_dataset_path = PathBuf::from(value);
                converted_overridden = true;
            }
            "--corpus-limit" => {
                let value = take_value("--corpus-limit", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --corpus-limit value '{value}' as usize")
                })?;
                config.corpus_limit = if parsed == 0 { None } else { Some(parsed) };
            }
            "--reseed-slice" => {
                config.reseed_slice = true;
            }
            "--slice-reset-ingestion" => {
                config.slice_reset_ingestion = true;
            }
            "--report-dir" => {
                let value = take_value("--report-dir", &mut args)?;
                config.report_dir = PathBuf::from(value);
            }
            "--k" => {
                let value = take_value("--k", &mut args)?;
                let parsed = value
                    .parse::<usize>()
                    .with_context(|| format!("failed to parse --k value '{value}' as usize"))?;
                if parsed == 0 {
                    return Err(anyhow!("--k must be greater than zero"));
                }
                config.k = parsed;
            }
            "--limit" => {
                let value = take_value("--limit", &mut args)?;
                let parsed = value
                    .parse::<usize>()
                    .with_context(|| format!("failed to parse --limit value '{value}' as usize"))?;
                config.limit = if parsed == 0 { None } else { Some(parsed) };
            }
            "--sample" => {
                let value = take_value("--sample", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --sample value '{value}' as usize")
                })?;
                config.summary_sample = parsed.max(1);
            }
            "--full-context" => {
                config.full_context = true;
            }
            "--chunk-min" => {
                let value = take_value("--chunk-min", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --chunk-min value '{value}' as usize")
                })?;
                config.retrieval.chunk_min_chars = parsed.max(1);
            }
            "--chunk-max" => {
                let value = take_value("--chunk-max", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --chunk-max value '{value}' as usize")
                })?;
                config.retrieval.chunk_max_chars = parsed.max(1);
            }
            "--chunk-vector-take" => {
                let value = take_value("--chunk-vector-take", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --chunk-vector-take value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--chunk-vector-take must be greater than zero"));
                }
                config.retrieval.chunk_vector_take = Some(parsed);
            }
            "--chunk-fts-take" => {
                let value = take_value("--chunk-fts-take", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --chunk-fts-take value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--chunk-fts-take must be greater than zero"));
                }
                config.retrieval.chunk_fts_take = Some(parsed);
            }
            "--chunk-token-budget" => {
                let value = take_value("--chunk-token-budget", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --chunk-token-budget value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--chunk-token-budget must be greater than zero"));
                }
                config.retrieval.chunk_token_budget = Some(parsed);
            }
            "--chunk-token-chars" => {
                let value = take_value("--chunk-token-chars", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --chunk-token-chars value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--chunk-token-chars must be greater than zero"));
                }
                config.retrieval.chunk_avg_chars_per_token = Some(parsed);
            }
            "--retrieval-strategy" => {
                let value = take_value("--retrieval-strategy", &mut args)?;
                let parsed = value.parse::<RetrievalStrategy>().map_err(|err| {
                    anyhow!("failed to parse --retrieval-strategy value '{value}': {err}")
                })?;
                config.retrieval.strategy = parsed;
            }
            "--max-chunks-per-entity" => {
                let value = take_value("--max-chunks-per-entity", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --max-chunks-per-entity value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--max-chunks-per-entity must be greater than zero"));
                }
                config.retrieval.max_chunks_per_entity = Some(parsed);
            }
            "--embedding" => {
                let value = take_value("--embedding", &mut args)?;
                config.embedding_backend = value.parse()?;
            }
            "--embedding-model" => {
                let value = take_value("--embedding-model", &mut args)?;
                config.embedding_model = Some(value.trim().to_string());
            }
            "--cache-dir" => {
                let value = take_value("--cache-dir", &mut args)?;
                config.cache_dir = PathBuf::from(value);
            }
            "--ingestion-cache-dir" => {
                let value = take_value("--ingestion-cache-dir", &mut args)?;
                config.ingestion_cache_dir = PathBuf::from(value);
            }
            "--ingestion-batch-size" => {
                let value = take_value("--ingestion-batch-size", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --ingestion-batch-size value '{value}' as usize")
                })?;
                if parsed == 0 {
                    return Err(anyhow!("--ingestion-batch-size must be greater than zero"));
                }
                config.ingestion_batch_size = parsed;
            }
            "--ingestion-max-retries" => {
                let value = take_value("--ingestion-max-retries", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --ingestion-max-retries value '{value}' as usize")
                })?;
                config.ingestion_max_retries = parsed;
            }
            "--negative-multiplier" => {
                let value = take_value("--negative-multiplier", &mut args)?;
                let parsed = value.parse::<f32>().with_context(|| {
                    format!("failed to parse --negative-multiplier value '{value}' as f32")
                })?;
                if !(parsed.is_finite() && parsed > 0.0) {
                    return Err(anyhow!(
                        "--negative-multiplier must be a positive finite number"
                    ));
                }
                config.negative_multiplier = parsed;
            }
            "--no-rerank" => {
                config.retrieval.rerank = false;
            }
            "--rerank-pool" => {
                let value = take_value("--rerank-pool", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --rerank-pool value '{value}' as usize")
                })?;
                config.retrieval.rerank_pool_size = parsed.max(1);
            }
            "--rerank-keep" => {
                let value = take_value("--rerank-keep", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --rerank-keep value '{value}' as usize")
                })?;
                config.retrieval.rerank_keep_top = parsed.max(1);
            }
            "--concurrency" => {
                let value = take_value("--concurrency", &mut args)?;
                let parsed = value.parse::<usize>().with_context(|| {
                    format!("failed to parse --concurrency value '{value}' as usize")
                })?;
                config.concurrency = parsed.max(1);
            }
            "--refresh-embeddings" => {
                config.refresh_embeddings_only = true;
            }
            "--detailed-report" => {
                config.detailed_report = true;
            }
            "--chunk-diagnostics" => {
                let value = take_value("--chunk-diagnostics", &mut args)?;
                config.chunk_diagnostics_path = Some(PathBuf::from(value));
            }
            "--inspect-question" => {
                let value = take_value("--inspect-question", &mut args)?;
                config.inspect_question = Some(value);
            }
            "--inspect-manifest" => {
                let value = take_value("--inspect-manifest", &mut args)?;
                config.inspect_manifest = Some(PathBuf::from(value));
            }
            "--inspect-db-state" => {
                let value = take_value("--inspect-db-state", &mut args)?;
                config.inspect_db_state = Some(PathBuf::from(value));
            }
            "--perf-log-json" => {
                let value = take_value("--perf-log-json", &mut args)?;
                config.perf_log_json = Some(PathBuf::from(value));
            }
            "--perf-log-dir" => {
                let value = take_value("--perf-log-dir", &mut args)?;
                config.perf_log_dir = Some(PathBuf::from(value));
            }
            "--perf-log" => {
                config.perf_log_console = true;
            }
            "--db-endpoint" => {
                let value = take_value("--db-endpoint", &mut args)?;
                config.db_endpoint = value;
            }
            "--db-user" => {
                let value = take_value("--db-user", &mut args)?;
                config.db_username = value;
            }
            "--db-pass" => {
                let value = take_value("--db-pass", &mut args)?;
                config.db_password = value;
            }
            "--db-namespace" => {
                let value = take_value("--db-namespace", &mut args)?;
                config.db_namespace = Some(value);
            }
            "--db-database" => {
                let value = take_value("--db-database", &mut args)?;
                config.db_database = Some(value);
            }
            unknown => {
                return Err(anyhow!(
                    "unknown argument '{unknown}'. Use --help to see available options."
                ));
            }
        }
    }

    if config.retrieval.chunk_min_chars >= config.retrieval.chunk_max_chars {
        return Err(anyhow!(
            "--chunk-min must be less than --chunk-max (got {} >= {})",
            config.retrieval.chunk_min_chars,
            config.retrieval.chunk_max_chars
        ));
    }

    if config.retrieval.rerank && config.retrieval.rerank_pool_size == 0 {
        return Err(anyhow!(
            "--rerank-pool must be greater than zero when reranking is enabled"
        ));
    }

    if config.concurrency == 0 {
        return Err(anyhow!("--concurrency must be greater than zero"));
    }

    if config.embedding_backend == EmbeddingBackend::Hashed && config.embedding_model.is_some() {
        return Err(anyhow!(
            "--embedding-model cannot be used with the 'hashed' embedding backend"
        ));
    }

    if let Some(limit) = config.limit {
        if let Some(corpus_limit) = config.corpus_limit {
            if corpus_limit < limit {
                config.corpus_limit = Some(limit);
            }
        } else {
            let default_multiplier = 10usize;
            let mut computed = limit.saturating_mul(default_multiplier);
            if computed < limit {
                computed = limit;
            }
            let max_cap = 1_000usize;
            if computed > max_cap {
                computed = max_cap;
            }
            config.corpus_limit = Some(computed);
        }
    }

    if config.perf_log_dir.is_none() {
        if let Ok(dir) = env::var("EVAL_PERF_LOG_DIR") {
            if !dir.trim().is_empty() {
                config.perf_log_dir = Some(PathBuf::from(dir));
            }
        }
    }

    if let Ok(endpoint) = env::var("EVAL_DB_ENDPOINT") {
        if !endpoint.trim().is_empty() {
            config.db_endpoint = endpoint;
        }
    }
    if let Ok(username) = env::var("EVAL_DB_USERNAME") {
        if !username.trim().is_empty() {
            config.db_username = username;
        }
    }
    if let Ok(password) = env::var("EVAL_DB_PASSWORD") {
        if !password.trim().is_empty() {
            config.db_password = password;
        }
    }
    if let Ok(ns) = env::var("EVAL_DB_NAMESPACE") {
        if !ns.trim().is_empty() {
            config.db_namespace = Some(ns);
        }
    }
    if let Ok(db) = env::var("EVAL_DB_DATABASE") {
        if !db.trim().is_empty() {
            config.db_database = Some(db);
        }
    }
    Ok(ParsedArgs { config, show_help })
}

fn take_value<'a, I>(flag: &str, iter: &mut std::iter::Peekable<I>) -> Result<String>
where
    I: Iterator<Item = String>,
{
    iter.next().ok_or_else(|| anyhow!("{flag} expects a value"))
}

pub fn print_help() {
    let report_default = default_report_dir();
    let cache_default = default_cache_dir();
    let ingestion_cache_default = default_ingestion_cache_dir();
    let report_default_display = report_default.display();
    let cache_default_display = cache_default.display();
    let ingestion_cache_default_display = ingestion_cache_default.display();

    println!(
        "\
eval — dataset conversion, ingestion, and retrieval evaluation CLI

USAGE:
    cargo eval -- [options]
    # or
    cargo run -p eval -- [options]

OPTIONS:
    --convert-only        Convert the selected dataset and exit.
    --force, --refresh    Regenerate the converted dataset even if it already exists.
    --dataset <name>      Dataset to evaluate: 'squad' (default) or 'natural-questions'.
    --llm-mode            Enable LLM-assisted evaluation features (includes unanswerable cases).
    --slice <id|path>     Use a cached dataset slice by id (under eval/cache/slices) or by explicit path.
    --label <text>        Annotate the run; label is stored in JSON/Markdown reports.
    --query-model <name>  Override the SurrealDB system settings query model (e.g., gpt-4o-mini) for this run.
    --slice-grow <int>    Grow the slice ledger to contain at least this many answerable cases, then exit.
    --slice-offset <int>  Evaluate questions starting at this offset within the slice (default: 0).
    --reseed-slice        Ignore cached corpus state and rebuild the slice's SurrealDB corpus.
    --slice-reset-ingestion
                          Delete cached paragraph shards before rebuilding the ingestion corpus.
    --corpus-limit <int>  Cap the slice corpus size (positives + negatives). Defaults to ~10× --limit, capped at 1000.
    --raw <path>          Path to the raw dataset (defaults per dataset).
    --converted <path>    Path to write/read the converted dataset (defaults per dataset).
    --report-dir <path>   Directory to write evaluation reports (default: {report_default_display}).
    --k <int>             Precision@k cutoff (default: 5).
    --limit <int>         Limit the number of questions evaluated (default: 200, 0 = all).
    --sample <int>        Number of mismatches to surface in the Markdown summary (default: 5).
    --full-context        Disable context cropping when converting datasets (ingest entire documents).
    --chunk-min <int>     Minimum characters per chunk for text splitting (default: 500).
    --chunk-max <int>     Maximum characters per chunk for text splitting (default: 2000).
    --chunk-vector-take <int>
                        Override chunk vector candidate cap (default: 20).
    --chunk-fts-take <int>
                        Override chunk FTS candidate cap (default: 20).
    --chunk-token-budget <int>
                        Override chunk token budget estimate for assembly (default: 10000).
    --chunk-token-chars <int>
                        Override average characters per token used for budgeting (default: 4).
    --retrieval-strategy <initial|revised>
                        Select the retrieval pipeline strategy (default: initial).
    --max-chunks-per-entity <int>
                        Override maximum chunks attached per entity (default: 4).
    --embedding <name>    Embedding backend: 'fastembed' (default) or 'hashed'.
    --embedding-model <code>
                          FastEmbed model code (defaults to crate preset when omitted).
    --cache-dir <path>    Directory for embedding caches (default: {cache_default_display}).
     --ingestion-cache-dir <path>
                          Directory for ingestion corpora caches (default: {ingestion_cache_default_display}).
     --ingestion-batch-size <int>
                          Number of paragraphs to ingest concurrently (default: 5).
     --ingestion-max-retries <int>
                          Maximum retries for ingestion failures per paragraph (default: 3).
    --negative-multiplier <float>
                          Target negative-to-positive paragraph ratio for slice growth (default: 4.0).
    --refresh-embeddings  Recompute embeddings for cached corpora without re-running ingestion.
    --detailed-report     Include entity descriptions and categories in JSON reports.
    --chunk-diagnostics <path>
                        Write per-query chunk diagnostics JSONL to the provided path.
    --no-rerank           Disable the FastEmbed reranking stage (enabled by default).
    --rerank-pool <int>   Reranking engine pool size / parallelism (default: 16).
    --rerank-keep <int>   Keep top-N entities after reranking (default: 10).
    --inspect-question <id>
                        Inspect an ingestion cache question and exit (requires --inspect-manifest).
    --inspect-manifest <path>
                        Path to an ingestion cache manifest JSON for inspection mode.
    --inspect-db-state <path>
                        Optional override for the SurrealDB state.json used during inspection; defaults to the state recorded for the selected dataset slice.
    --db-endpoint <url>  SurrealDB server endpoint (use http:// or https:// to enable SurQL export/import; ws:// endpoints reuse existing namespaces but skip SurQL exports; default: ws://127.0.0.1:8000).
    --db-user <value>    SurrealDB root username (default: root_user).
    --db-pass <value>    SurrealDB root password (default: root_password).
    --db-namespace <ns>  Override the namespace used on the SurrealDB server; state.json tracks this value and the ledger case count so changing it or requesting more cases via --limit triggers a rebuild/import (default: derived from dataset).
    --db-database <db>   Override the database used on the SurrealDB server; recorded alongside namespace in state.json (default: derived from slice).
    --perf-log           Print per-stage performance timings to stdout after the run.
    --perf-log-json <path>
                        Write structured performance telemetry JSON to the provided path.
    --perf-log-dir <path>
                        Directory that receives timestamped perf JSON copies (defaults to $EVAL_PERF_LOG_DIR).

Examples:
    cargo eval -- --dataset squad --limit 10 --detailed-report
    cargo eval -- --dataset natural-questions --limit 1 --rerank-pool 1 --detailed-report

Notes:
    The latest run's JSON/Markdown reports are saved as eval/reports/latest.json and latest.md, making it easy to script automated checks.
    -h, --help            Show this help text.

Dataset defaults (from eval/manifest.yaml):
    squad               raw: eval/data/raw/squad/dev-v2.0.json
                        converted: eval/data/converted/squad-minne.json
    natural-questions   raw: eval/data/raw/nq/dev-all.jsonl
                        converted: eval/data/converted/nq-dev-minne.json
"
    );
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory for {}", path.display()))?;
    }
    Ok(())
}
