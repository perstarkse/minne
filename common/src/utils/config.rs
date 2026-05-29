use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Deserializer, Serialize};
use std::{env, fmt, str::FromStr, sync::Once};
use thiserror::Error;
use tracing::warn;

/// Error returned when parsing an embedding backend name.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("unknown embedding backend '{input}': expected 'openai', 'hashed', or 'fastembed'")]
pub struct ParseEmbeddingBackendError {
    /// The unrecognized input string.
    pub input: String,
}

/// Selects the embedding backend for vector generation.
#[derive(Clone, Copy, Deserialize, Serialize, Debug, Default, PartialEq, Eq)]
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

impl EmbeddingBackend {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::FastEmbed => "fastembed",
            Self::Hashed => "hashed",
        }
    }
}

/// Error returned when parsing a retrieval strategy name.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("unknown retrieval strategy '{input}'")]
pub struct ParseRetrievalStrategyError {
    /// The unrecognized input string.
    pub input: String,
}

/// Selects which retrieval pipeline strategy to run for chat and search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStrategy {
    /// Primary hybrid chunk retrieval for search/chat.
    #[default]
    Default,
    /// Entity retrieval for suggesting relationships when creating manual entities.
    RelationshipSuggestion,
    /// Entity retrieval for context during content ingestion.
    Ingestion,
    /// Unified search returning both chunks and entities.
    Search,
}

impl RetrievalStrategy {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::RelationshipSuggestion => "relationship_suggestion",
            Self::Ingestion => "ingestion",
            Self::Search => "search",
        }
    }
}

impl FromStr for RetrievalStrategy {
    type Err = ParseRetrievalStrategyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "initial" | "revised" => {
                warn!(
                    "retrieval strategy '{value}' is deprecated; use 'default' instead"
                );
                Ok(Self::Default)
            }
            "relationship_suggestion" => Ok(Self::RelationshipSuggestion),
            "ingestion" => Ok(Self::Ingestion),
            "search" => Ok(Self::Search),
            other => Err(ParseRetrievalStrategyError {
                input: other.to_string(),
            }),
        }
    }
}

impl fmt::Display for RetrievalStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn deserialize_optional_retrieval_strategy<'de, D>(
    deserializer: D,
) -> Result<Option<RetrievalStrategy>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Ok(None),
        Some(raw) => RetrievalStrategy::from_str(&raw).map(Some).map_err(serde::de::Error::custom),
    }
}

impl FromStr for EmbeddingBackend {
    type Err = ParseEmbeddingBackendError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "hashed" => Ok(Self::Hashed),
            "fastembed" | "fast-embed" | "fast" => Ok(Self::FastEmbed),
            other => Err(ParseEmbeddingBackendError {
                input: other.to_string(),
            }),
        }
    }
}

#[derive(Clone, Copy, Deserialize, Debug, PartialEq)]
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

fn default_s3_region() -> String {
    "us-east-1".to_string()
}

/// Selects the strategy used for PDF ingestion.
#[derive(Clone, Copy, Deserialize, Debug)]
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
    pub s3_region: String,
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
    #[serde(default, deserialize_with = "deserialize_optional_retrieval_strategy")]
    pub retrieval_strategy: Option<RetrievalStrategy>,
    #[serde(default)]
    pub embedding_backend: EmbeddingBackend,
    #[serde(default = "default_ingest_max_body_bytes")]
    pub ingest_max_body_bytes: usize,
    #[serde(default = "default_ingest_max_files")]
    pub ingest_max_files: usize,
    #[serde(default = "default_ingest_max_content_bytes")]
    pub ingest_max_content_bytes: usize,
    #[serde(default = "default_ingest_max_context_bytes")]
    pub ingest_max_context_bytes: usize,
    #[serde(default = "default_ingest_max_category_bytes")]
    pub ingest_max_category_bytes: usize,
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

fn default_ingest_max_body_bytes() -> usize {
    20_000_000
}

fn default_ingest_max_files() -> usize {
    5
}

fn default_ingest_max_content_bytes() -> usize {
    262_144
}

fn default_ingest_max_context_bytes() -> usize {
    16_384
}

fn default_ingest_max_category_bytes() -> usize {
    128
}

static ORT_PATH_INIT: Once = Once::new();

/// Sets `ORT_DYLIB_PATH` once per process when a bundled ONNX runtime library is found.
pub fn ensure_ort_path() {
    ORT_PATH_INIT.call_once(|| {
        if env::var_os("ORT_DYLIB_PATH").is_some() {
            return;
        }
        let Ok(mut exe) = env::current_exe() else {
            return;
        };
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
    });
}

impl AppConfig {
    /// Returns the configured retrieval strategy, or [`RetrievalStrategy::Default`] when unset.
    #[must_use]
    pub fn resolved_retrieval_strategy(&self) -> RetrievalStrategy {
        self.retrieval_strategy.unwrap_or_default()
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
            ingest_max_body_bytes: default_ingest_max_body_bytes(),
            ingest_max_files: default_ingest_max_files(),
            ingest_max_content_bytes: default_ingest_max_content_bytes(),
            ingest_max_context_bytes: default_ingest_max_context_bytes(),
            ingest_max_category_bytes: default_ingest_max_category_bytes(),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{ParseRetrievalStrategyError, RetrievalStrategy};
    #[test]
    fn retrieval_strategy_defaults_to_default() {
        assert_eq!(
            RetrievalStrategy::default(),
            RetrievalStrategy::Default
        );
    }

    #[test]
    fn retrieval_strategy_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&RetrievalStrategy::Search).expect("serialize"),
            "\"search\""
        );
    }

    #[test]
    fn retrieval_strategy_from_str_accepts_deprecated_aliases() {
        assert_eq!(
            "initial".parse::<RetrievalStrategy>().expect("initial"),
            RetrievalStrategy::Default
        );
        assert!(matches!(
            "unknown".parse::<RetrievalStrategy>(),
            Err(ParseRetrievalStrategyError { .. })
        ));
    }

    #[test]
    fn app_config_resolved_retrieval_strategy_uses_default_when_unset() {
        let config = super::AppConfig::default();
        assert_eq!(
            config.resolved_retrieval_strategy(),
            RetrievalStrategy::Default
        );
    }
}
