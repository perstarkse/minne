use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::env;

/// Selects the embedding backend for vector generation.
#[derive(Clone, Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingBackend {
    /// Use OpenAI-compatible API for embeddings.
    OpenAI,
    /// Use FastEmbed local embeddings (default).
    #[default]
    FastEmbed,
    /// Use deterministic hashed embeddings (for testing).
    Hashed,
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StorageKind {
    Local,
    Memory,
    S3,
}

/// Default storage backend when none is configured.
fn default_storage_kind() -> StorageKind {
    StorageKind::Local
}

fn default_s3_region() -> Option<String> {
    Some("us-east-1".to_string())
}

/// Selects the strategy used for PDF ingestion.
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum PdfIngestMode {
    /// Only rely on classic text extraction (no LLM fallbacks).
    Classic,
    /// Prefer fast text extraction, but fall back to the LLM rendering path when needed.
    LlmFirst,
}

/// Default PDF ingestion mode when unset.
fn default_pdf_ingest_mode() -> PdfIngestMode {
    PdfIngestMode::LlmFirst
}

/// Application configuration loaded from files and environment variables.
#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Deserialize, Debug)]
pub struct AppConfig {
    pub openai_api_key: String,
    pub surrealdb_address: String,
    pub surrealdb_username: String,
    pub surrealdb_password: String,
    pub surrealdb_namespace: String,
    pub surrealdb_database: String,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    pub http_port: u16,
    #[serde(default = "default_base_url")]
    pub openai_base_url: String,
    #[serde(default = "default_storage_kind")]
    pub storage: StorageKind,
    #[serde(default)]
    pub s3_bucket: Option<String>,
    #[serde(default)]
    pub s3_endpoint: Option<String>,
    #[serde(default = "default_s3_region")]
    pub s3_region: Option<String>,
    #[serde(default = "default_pdf_ingest_mode")]
    pub pdf_ingest_mode: PdfIngestMode,
    #[serde(default = "default_reranking_enabled")]
    pub reranking_enabled: bool,
    #[serde(default)]
    pub reranking_pool_size: Option<usize>,
    #[serde(default)]
    pub fastembed_cache_dir: Option<String>,
    #[serde(default)]
    pub fastembed_show_download_progress: Option<bool>,
    #[serde(default)]
    pub fastembed_max_length: Option<usize>,
    #[serde(default)]
    pub retrieval_strategy: Option<String>,
    #[serde(default)]
    pub embedding_backend: EmbeddingBackend,
}

/// Default data directory for persisted assets.
fn default_data_dir() -> String {
    "./data".to_string()
}

/// Default base URL used for OpenAI-compatible APIs.
fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

/// Whether reranking is enabled by default.
fn default_reranking_enabled() -> bool {
    false
}

pub fn ensure_ort_path() {
    if env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    if let Ok(mut exe) = env::current_exe() {
        exe.pop();

        if cfg!(target_os = "windows") {
            for p in [
                exe.join("onnxruntime.dll"),
                exe.join("lib").join("onnxruntime.dll"),
            ] {
                if p.exists() {
                    env::set_var("ORT_DYLIB_PATH", p);
                    return;
                }
            }
        }
        let name = if cfg!(target_os = "macos") {
            "libonnxruntime.dylib"
        } else {
            "libonnxruntime.so"
        };
        let p = exe.join("lib").join(name);
        if p.exists() {
            env::set_var("ORT_DYLIB_PATH", p);
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            openai_api_key: String::new(),
            surrealdb_address: String::new(),
            surrealdb_username: String::new(),
            surrealdb_password: String::new(),
            surrealdb_namespace: String::new(),
            surrealdb_database: String::new(),
            data_dir: default_data_dir(),
            http_port: 0,
            openai_base_url: default_base_url(),
            storage: default_storage_kind(),
            s3_bucket: None,
            s3_endpoint: None,
            s3_region: default_s3_region(),
            pdf_ingest_mode: default_pdf_ingest_mode(),
            reranking_enabled: default_reranking_enabled(),
            reranking_pool_size: None,
            fastembed_cache_dir: None,
            fastembed_show_download_progress: None,
            fastembed_max_length: None,
            retrieval_strategy: None,
            embedding_backend: EmbeddingBackend::default(),
        }
    }
}

/// Loads the application configuration from the environment and optional config file.
#[allow(clippy::module_name_repetitions)]
pub fn get_config() -> Result<AppConfig, ConfigError> {
    ensure_ort_path();

    let config = Config::builder()
        .add_source(File::with_name("config").required(false))
        .add_source(Environment::default())
        .build()?;

    config.try_deserialize()
}
