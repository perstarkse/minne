use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, ValueEnum};
use retrieval_pipeline::RetrievalStrategy;

use crate::datasets::DatasetKind;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf()
}

fn default_report_dir() -> PathBuf {
    workspace_root().join("evaluations/reports")
}

fn default_cache_dir() -> PathBuf {
    workspace_root().join("evaluations/cache")
}

fn default_ingestion_cache_dir() -> PathBuf {
    workspace_root().join("evaluations/cache/ingested")
}

pub const DEFAULT_SLICE_SEED: u64 = 0x5eed_2025;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum EmbeddingBackend {
    Hashed,
    FastEmbed,
}

impl Default for EmbeddingBackend {
    fn default() -> Self {
        Self::FastEmbed
    }
}

impl std::fmt::Display for EmbeddingBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hashed => write!(f, "hashed"),
            Self::FastEmbed => write!(f, "fastembed"),
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct RetrievalSettings {
    /// Override chunk vector candidate cap
    #[arg(long)]
    pub chunk_vector_take: Option<usize>,

    /// Override chunk FTS candidate cap
    #[arg(long)]
    pub chunk_fts_take: Option<usize>,

    /// Override average characters per token used for budgeting
    #[arg(long)]
    pub chunk_avg_chars_per_token: Option<usize>,

    /// Override maximum chunks attached per entity
    #[arg(long)]
    pub max_chunks_per_entity: Option<usize>,

    /// Enable the FastEmbed reranking stage
    #[arg(long = "rerank", action = clap::ArgAction::SetTrue, default_value_t = false)]
    pub rerank: bool,

    /// Reranking engine pool size / parallelism
    #[arg(long, default_value_t = 4)]
    pub rerank_pool_size: usize,

    /// Keep top-N entities after reranking
    #[arg(long, default_value_t = 10)]
    pub rerank_keep_top: usize,

    /// Cap the number of chunks returned by retrieval (revised strategy)
    #[arg(long, default_value_t = 5)]
    pub chunk_result_cap: usize,

    /// Reciprocal rank fusion k value for revised chunk merging
    #[arg(long)]
    pub chunk_rrf_k: Option<f32>,

    /// Weight for vector ranks in revised RRF
    #[arg(long)]
    pub chunk_rrf_vector_weight: Option<f32>,

    /// Weight for chunk FTS ranks in revised RRF
    #[arg(long)]
    pub chunk_rrf_fts_weight: Option<f32>,

    /// Include vector ranks in revised RRF (default: true)
    #[arg(long)]
    pub chunk_rrf_use_vector: Option<bool>,

    /// Include chunk FTS ranks in revised RRF (default: true)
    #[arg(long)]
    pub chunk_rrf_use_fts: Option<bool>,

    /// Require verified chunks (disable with --llm-mode)
    #[arg(skip = true)]
    pub require_verified_chunks: bool,

    /// Select the retrieval pipeline strategy
    #[arg(long, default_value_t = RetrievalStrategy::Initial)]
    pub strategy: RetrievalStrategy,
}

impl Default for RetrievalSettings {
    fn default() -> Self {
        Self {
            chunk_vector_take: None,
            chunk_fts_take: None,
            chunk_avg_chars_per_token: None,
            max_chunks_per_entity: None,
            rerank: false,
            rerank_pool_size: 4,
            rerank_keep_top: 10,
            chunk_result_cap: 5,
            chunk_rrf_k: None,
            chunk_rrf_vector_weight: None,
            chunk_rrf_fts_weight: None,
            chunk_rrf_use_vector: None,
            chunk_rrf_use_fts: None,
            require_verified_chunks: true,
            strategy: RetrievalStrategy::Initial,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct IngestConfig {
    /// Directory for ingestion corpora caches
    #[arg(long, default_value_os_t = default_ingestion_cache_dir())]
    pub ingestion_cache_dir: PathBuf,

    /// Minimum tokens per chunk for ingestion
    #[arg(long, default_value_t = 256)]
    pub ingest_chunk_min_tokens: usize,

    /// Maximum tokens per chunk for ingestion
    #[arg(long, default_value_t = 512)]
    pub ingest_chunk_max_tokens: usize,

    /// Overlap between chunks during ingestion (tokens)
    #[arg(long, default_value_t = 50)]
    pub ingest_chunk_overlap_tokens: usize,

    /// Run ingestion in chunk-only mode (skip analyzer/graph generation)
    #[arg(long)]
    pub ingest_chunks_only: bool,

    /// Number of paragraphs to ingest concurrently
    #[arg(long, default_value_t = 10)]
    pub ingestion_batch_size: usize,

    /// Maximum retries for ingestion failures per paragraph
    #[arg(long, default_value_t = 3)]
    pub ingestion_max_retries: usize,

    /// Recompute embeddings for cached corpora without re-running ingestion
    #[arg(long, alias = "refresh-embeddings")]
    pub refresh_embeddings_only: bool,

    /// Delete cached paragraph shards before rebuilding the ingestion corpus
    #[arg(long)]
    pub slice_reset_ingestion: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DatabaseArgs {
    /// SurrealDB server endpoint
    #[arg(long, default_value = "ws://127.0.0.1:8000", env = "EVAL_DB_ENDPOINT")]
    pub db_endpoint: String,

    /// SurrealDB root username
    #[arg(long, default_value = "root_user", env = "EVAL_DB_USERNAME")]
    pub db_username: String,

    /// SurrealDB root password
    #[arg(long, default_value = "root_password", env = "EVAL_DB_PASSWORD")]
    pub db_password: String,

    /// Override the namespace used on the SurrealDB server
    #[arg(long, env = "EVAL_DB_NAMESPACE")]
    pub db_namespace: Option<String>,

    /// Override the database used on the SurrealDB server
    #[arg(long, env = "EVAL_DB_DATABASE")]
    pub db_database: Option<String>,

    /// Path to inspect DB state
    #[arg(long)]
    pub inspect_db_state: Option<PathBuf>,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Convert the selected dataset and exit
    #[arg(long)]
    pub convert_only: bool,

    /// Regenerate the converted dataset even if it already exists
    #[arg(long, alias = "refresh")]
    pub force_convert: bool,

    /// Dataset to evaluate
    #[arg(long, default_value_t = DatasetKind::default())]
    pub dataset: DatasetKind,

    /// Enable LLM-assisted evaluation features (includes unanswerable cases)
    #[arg(long)]
    pub llm_mode: bool,

    /// Cap the slice corpus size (positives + negatives)
    #[arg(long)]
    pub corpus_limit: Option<usize>,

    /// Path to the raw dataset (defaults per dataset)
    #[arg(long)]
    pub raw: Option<PathBuf>,

    /// Path to write/read the converted dataset (defaults per dataset)
    #[arg(long)]
    pub converted: Option<PathBuf>,

    /// Directory to write evaluation reports
    #[arg(long, default_value_os_t = default_report_dir())]
    pub report_dir: PathBuf,

    /// Precision@k cutoff
    #[arg(long, default_value_t = 5)]
    pub k: usize,

    /// Limit the number of questions evaluated (0 = all)
    #[arg(long = "limit", default_value_t = 200)]
    pub limit_arg: usize,

    /// Number of mismatches to surface in the Markdown summary
    #[arg(long, default_value_t = 5)]
    pub sample: usize,

    /// Disable context cropping when converting datasets (ingest entire documents)
    #[arg(long)]
    pub full_context: bool,

    #[command(flatten)]
    pub retrieval: RetrievalSettings,

    /// Concurrency level
    #[arg(long, default_value_t = 1)]
    pub concurrency: usize,

    /// Embedding backend
    #[arg(long, default_value_t = EmbeddingBackend::FastEmbed)]
    pub embedding_backend: EmbeddingBackend,

    /// FastEmbed model code
    #[arg(long)]
    pub embedding_model: Option<String>,

    /// Directory for embedding caches
    #[arg(long, default_value_os_t = default_cache_dir())]
    pub cache_dir: PathBuf,

    #[command(flatten)]
    pub ingest: IngestConfig,

    /// Include entity descriptions and categories in JSON reports
    #[arg(long)]
    pub detailed_report: bool,

    /// Use a cached dataset slice by id or path
    #[arg(long)]
    pub slice: Option<String>,

    /// Ignore cached corpus state and rebuild the slice's SurrealDB corpus
    #[arg(long)]
    pub reseed_slice: bool,

    /// Slice seed
    #[arg(skip = DEFAULT_SLICE_SEED)]
    pub slice_seed: u64,

    /// Grow the slice ledger to contain at least this many answerable cases, then exit
    #[arg(long)]
    pub slice_grow: Option<usize>,

    /// Evaluate questions starting at this offset within the slice
    #[arg(long, default_value_t = 0)]
    pub slice_offset: usize,

    /// Target negative-to-positive paragraph ratio for slice growth
    #[arg(long, default_value_t = crate::slice::DEFAULT_NEGATIVE_MULTIPLIER)]
    pub negative_multiplier: f32,

    /// Annotate the run; label is stored in JSON/Markdown reports
    #[arg(long)]
    pub label: Option<String>,

    /// Write per-query chunk diagnostics JSONL to the provided path
    #[arg(long, alias = "chunk-diagnostics")]
    pub chunk_diagnostics_path: Option<PathBuf>,

    /// Inspect an ingestion cache question and exit
    #[arg(long)]
    pub inspect_question: Option<String>,

    /// Path to an ingestion cache manifest JSON for inspection mode
    #[arg(long)]
    pub inspect_manifest: Option<PathBuf>,

    /// Override the SurrealDB system settings query model
    #[arg(long)]
    pub query_model: Option<String>,

    /// Write structured performance telemetry JSON to the provided path
    #[arg(long)]
    pub perf_log_json: Option<PathBuf>,

    /// Directory that receives timestamped perf JSON copies
    #[arg(long)]
    pub perf_log_dir: Option<PathBuf>,

    /// Print per-stage performance timings to stdout after the run
    #[arg(long, alias = "perf-log")]
    pub perf_log_console: bool,

    #[command(flatten)]
    pub database: DatabaseArgs,

    // Computed fields (not arguments)
    #[arg(skip)]
    pub raw_dataset_path: PathBuf,
    #[arg(skip)]
    pub converted_dataset_path: PathBuf,
    #[arg(skip)]
    pub limit: Option<usize>,
    #[arg(skip)]
    pub summary_sample: usize,
}

impl Config {
    pub fn context_token_limit(&self) -> Option<usize> {
        None
    }

    pub fn finalize(&mut self) -> Result<()> {
        // Handle dataset paths
        if let Some(raw) = &self.raw {
            self.raw_dataset_path = raw.clone();
        } else {
            self.raw_dataset_path = self.dataset.default_raw_path();
        }

        if let Some(converted) = &self.converted {
            self.converted_dataset_path = converted.clone();
        } else {
            self.converted_dataset_path = self.dataset.default_converted_path();
        }

        // Handle limit
        if self.limit_arg == 0 {
            self.limit = None;
        } else {
            self.limit = Some(self.limit_arg);
        }

        // Handle sample
        self.summary_sample = self.sample.max(1);

        // Handle retrieval settings
        if self.llm_mode {
            self.retrieval.require_verified_chunks = false;
        } else {
            self.retrieval.require_verified_chunks = true;
        }

        if self.dataset == DatasetKind::Beir {
            self.negative_multiplier = 9.0;
        }

        // Validations
        if self.ingest.ingest_chunk_min_tokens == 0
            || self.ingest.ingest_chunk_min_tokens >= self.ingest.ingest_chunk_max_tokens
        {
            return Err(anyhow!(
                "--ingest-chunk-min-tokens must be greater than zero and less than --ingest-chunk-max-tokens (got {} >= {})",
                self.ingest.ingest_chunk_min_tokens,
                self.ingest.ingest_chunk_max_tokens
            ));
        }

        if self.ingest.ingest_chunk_overlap_tokens >= self.ingest.ingest_chunk_min_tokens {
            return Err(anyhow!(
                "--ingest-chunk-overlap-tokens ({}) must be less than --ingest-chunk-min-tokens ({})",
                self.ingest.ingest_chunk_overlap_tokens,
                self.ingest.ingest_chunk_min_tokens
            ));
        }

        if self.retrieval.rerank && self.retrieval.rerank_pool_size == 0 {
            return Err(anyhow!(
                "--rerank-pool must be greater than zero when reranking is enabled"
            ));
        }

        if let Some(k) = self.retrieval.chunk_rrf_k {
            if k <= 0.0 || !k.is_finite() {
                return Err(anyhow!(
                    "--chunk-rrf-k must be a positive, finite number (got {k})"
                ));
            }
        }
        if let Some(weight) = self.retrieval.chunk_rrf_vector_weight {
            if weight < 0.0 || !weight.is_finite() {
                return Err(anyhow!(
                    "--chunk-rrf-vector-weight must be a non-negative, finite number (got {weight})"
                ));
            }
        }
        if let Some(weight) = self.retrieval.chunk_rrf_fts_weight {
            if weight < 0.0 || !weight.is_finite() {
                return Err(anyhow!(
                    "--chunk-rrf-fts-weight must be a non-negative, finite number (got {weight})"
                ));
            }
        }

        if self.concurrency == 0 {
            return Err(anyhow!("--concurrency must be greater than zero"));
        }

        if self.embedding_backend == EmbeddingBackend::Hashed && self.embedding_model.is_some() {
            return Err(anyhow!(
                "--embedding-model cannot be used with the 'hashed' embedding backend"
            ));
        }

        if let Some(query_model) = &self.query_model {
            if query_model.trim().is_empty() {
                return Err(anyhow!("--query-model requires a non-empty model name"));
            }
        }

        if let Some(grow) = self.slice_grow {
            if grow == 0 {
                return Err(anyhow!("--slice-grow must be greater than zero"));
            }
        }

        if self.negative_multiplier <= 0.0 || !self.negative_multiplier.is_finite() {
            return Err(anyhow!(
                "--negative-multiplier must be a positive finite number"
            ));
        }

        // Handle corpus limit logic
        if let Some(limit) = self.limit {
            if let Some(corpus_limit) = self.corpus_limit {
                if corpus_limit < limit {
                    self.corpus_limit = Some(limit);
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
                self.corpus_limit = Some(computed);
            }
        }

        // Handle perf log dir env var fallback
        if self.perf_log_dir.is_none() {
            if let Ok(dir) = env::var("EVAL_PERF_LOG_DIR") {
                if !dir.trim().is_empty() {
                    self.perf_log_dir = Some(PathBuf::from(dir));
                }
            }
        }

        Ok(())
    }
}

pub struct ParsedArgs {
    pub config: Config,
}

pub fn parse() -> Result<ParsedArgs> {
    let mut config = Config::parse();
    config.finalize()?;
    Ok(ParsedArgs { config })
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory for {}", path.display()))?;
    }
    Ok(())
}
