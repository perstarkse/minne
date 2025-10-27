use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum StorageKind {
    Local,
}

fn default_storage_kind() -> StorageKind {
    StorageKind::Local
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

fn default_pdf_ingest_mode() -> PdfIngestMode {
    PdfIngestMode::LlmFirst
}

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
}

fn default_data_dir() -> String {
    "./data".to_string()
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_reranking_enabled() -> bool {
    false
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
            pdf_ingest_mode: default_pdf_ingest_mode(),
            reranking_enabled: default_reranking_enabled(),
            reranking_pool_size: None,
            fastembed_cache_dir: None,
            fastembed_show_download_progress: None,
            fastembed_max_length: None,
        }
    }
}

pub fn get_config() -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        .add_source(File::with_name("config").required(false))
        .add_source(Environment::default())
        .build()?;

    config.try_deserialize()
}
