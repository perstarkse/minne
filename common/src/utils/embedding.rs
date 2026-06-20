use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::{Arc, Mutex},
    thread::available_parallelism,
};

use serde::Serialize;
use tracing::warn;

use async_openai::{types::embeddings::CreateEmbeddingRequestArgs, Client};
use fastembed::{EmbeddingModel, ModelTrait, TextEmbedding, TextInitOptions};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    error::{AppError, EmbeddingError},
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
    utils::config::AppConfig,
};

#[allow(clippy::module_name_repetitions)]
pub use crate::utils::config::{EmbeddingBackend, ParseEmbeddingBackendError};

/// Wrapper around the chosen embedding backend.
#[allow(clippy::module_name_repetitions)]
#[derive(Clone)]
pub struct EmbeddingProvider {
    /// Concrete backend implementation.
    inner: EmbeddingInner,
}

/// Concrete embedding implementations.
#[derive(Clone)]
enum EmbeddingInner {
    /// Uses an `OpenAI`-compatible API.
    OpenAI {
        /// Client used to issue embedding requests.
        client: Arc<Client<async_openai::config::OpenAIConfig>>,
        /// Model identifier for the API.
        model: Arc<str>,
        /// Expected output dimensions.
        dimensions: u32,
    },
    /// Generates deterministic hashed embeddings without external calls.
    Hashed {
        /// Output vector length.
        dimension: usize,
    },
    /// Uses `FastEmbed` running locally.
    FastEmbed {
        /// Pool of `FastEmbed` engines providing bounded-concurrency local embedding.
        pool: Arc<FastEmbedPool>,
        /// Model metadata used for info logging.
        model_name: EmbeddingModel,
        /// Output vector length.
        dimension: usize,
    },
}

/// Batch size used when re-embedding stored data in bulk. Bounds peak memory and preserves
/// progress logging while still amortising per-call lock/dispatch overhead.
pub const RE_EMBED_BATCH_SIZE: usize = 128;

/// Default FastEmbed model (`BGESmallENV15`) when config and DB do not specify a valid code.
pub const DEFAULT_FASTEMBED_MODEL_CODE: &str = "Xenova/bge-small-en-v1.5";

/// A supported FastEmbed model for admin UI and documentation.
#[derive(Clone, Debug, Serialize)]
pub struct FastEmbedModelOption {
    /// HuggingFace-style `model_code` accepted by [`EmbeddingModel::from_str`].
    pub model_code: String,
    /// Fixed output dimension for this model.
    pub dimension: u32,
    /// Short human-readable description from fastembed metadata.
    pub description: String,
}

/// Lists supported FastEmbed text embedding models (sorted by `model_code`).
#[must_use]
pub fn list_fastembed_embedding_models() -> Vec<FastEmbedModelOption> {
    let mut list: Vec<FastEmbedModelOption> = TextEmbedding::list_supported_models()
        .into_iter()
        .filter_map(|info| {
            let dimension = u32::try_from(info.dim).ok()?;
            Some(FastEmbedModelOption {
                model_code: info.model_code,
                dimension,
                description: info.description,
            })
        })
        .collect();
    list.sort_by(|left, right| left.model_code.cmp(&right.model_code));
    list
}

/// Returns true when `code` is a supported FastEmbed `model_code` (HuggingFace-style id).
#[must_use]
pub fn is_valid_fastembed_model_code(code: &str) -> bool {
    !code.trim().is_empty() && EmbeddingModel::from_str(code.trim()).is_ok()
}

/// Vector dimension for a supported FastEmbed `model_code`.
///
/// # Errors
///
/// Returns [`EmbeddingError::UnknownModel`] when the code is not recognized.
pub fn fastembed_model_dimension(code: &str) -> Result<u32, EmbeddingError> {
    let model = EmbeddingModel::from_str(code.trim())
        .map_err(|_| EmbeddingError::UnknownModel(unknown_fastembed_model_message(code)))?;
    let dim = EmbeddingModel::get_model_info(&model)
        .ok_or_else(|| {
            EmbeddingError::Config(format!("fastembed model metadata missing for {code}"))
        })?
        .dim;
    u32::try_from(dim).map_err(|_| {
        EmbeddingError::Config(format!("fastembed model dimension {dim} exceeds u32::MAX"))
    })
}

/// Resolves the FastEmbed model code to load: config override, then DB, then default.
///
/// When `config.fastembed_model` is set it must be valid. When only the DB value is used and it
/// is not a FastEmbed code (e.g. legacy `text-embedding-3-small`), returns the default model.
///
/// # Errors
///
/// Returns [`EmbeddingError::UnknownModel`] if `config.fastembed_model` is set but invalid.
pub fn resolve_fastembed_model_code(
    config: &AppConfig,
    settings_model: &str,
) -> Result<String, EmbeddingError> {
    if let Some(code) = config.fastembed_model.as_deref() {
        let trimmed = code.trim();
        if trimmed.is_empty() {
            return Err(EmbeddingError::Config(
                "fastembed_model must not be empty when set".into(),
            ));
        }
        EmbeddingModel::from_str(trimmed)
            .map_err(|_| EmbeddingError::UnknownModel(unknown_fastembed_model_message(trimmed)))?;
        return Ok(trimmed.to_owned());
    }

    let trimmed = settings_model.trim();
    if is_valid_fastembed_model_code(trimmed) {
        return Ok(trimmed.to_owned());
    }

    if !trimmed.is_empty() {
        warn!(
            stored_model = trimmed,
            default_model = DEFAULT_FASTEMBED_MODEL_CODE,
            "system_settings.embedding_model is not a FastEmbed model code; using default"
        );
    }

    Ok(DEFAULT_FASTEMBED_MODEL_CODE.to_owned())
}

/// Persists a FastEmbed-compatible `embedding_model` and `embedding_dimensions` before startup
/// when the active backend is FastEmbed and stored settings still carry OpenAI defaults.
///
/// # Errors
///
/// Returns [`AppError`] if settings cannot be loaded, resolved, or updated.
pub async fn align_fastembed_system_settings(
    db: &SurrealDbClient,
    config: &AppConfig,
) -> Result<SystemSettings, AppError> {
    if config.embedding_backend != EmbeddingBackend::FastEmbed {
        return SystemSettings::get_current(db).await;
    }

    let mut settings = SystemSettings::get_current(db).await?;
    let resolved = resolve_fastembed_model_code(config, &settings.embedding_model)?;
    let dimension = fastembed_model_dimension(&resolved)?;

    if settings.embedding_model == resolved && settings.embedding_dimensions == dimension {
        return Ok(settings);
    }

    tracing::info!(
        old_model = %settings.embedding_model,
        new_model = %resolved,
        old_dimensions = settings.embedding_dimensions,
        new_dimensions = dimension,
        "Aligning system settings with FastEmbed model"
    );
    settings.embedding_model = resolved;
    settings.embedding_dimensions = dimension;
    SystemSettings::update(db, settings).await
}

fn unknown_fastembed_model_message(code: &str) -> String {
    let mut codes: Vec<String> = TextEmbedding::list_supported_models()
        .into_iter()
        .map(|info| info.model_code)
        .collect();
    codes.sort();
    let examples: Vec<&str> = codes.iter().take(6).map(String::as_str).collect();
    format!(
        "unknown FastEmbed model '{code}' (expected a HuggingFace model_code such as {}). \
         Set fastembed_model in config.yaml or update system_settings; \
         see docs/configuration.md ({count} models supported)",
        examples.join(", "),
        count = codes.len()
    )
}

/// Default FastEmbed pool size.
///
/// Kept small on purpose: the ONNX runtime already uses intra-op threads per inference, so
/// running many engines concurrently oversubscribes the CPU and each engine duplicates the
/// model weights in memory. Mirrors the reranker pool default.
#[must_use]
pub fn default_embedding_pool_size() -> usize {
    available_parallelism()
        .map_or(2, |value| value.get().min(2))
        .max(1)
}

/// Pool of `FastEmbed` engines enabling bounded-concurrency local embedding.
///
/// A single [`TextEmbedding`] embeds one batch at a time (`&mut self`), so the pool keeps
/// several instances and hands out a distinct idle engine per checkout. The semaphore bounds
/// total in-flight embeds (backpressure); the free list guarantees each active lease holds a
/// different engine — unlike a round-robin index, which can hand the same engine to two callers.
struct FastEmbedPool {
    /// Idle engines; one is popped on checkout and returned on lease drop.
    engines: Mutex<Vec<Arc<Mutex<TextEmbedding>>>>,
    /// Sized to the engine count; gates concurrent checkouts.
    semaphore: Arc<Semaphore>,
}

impl FastEmbedPool {
    fn new(engines: Vec<Arc<Mutex<TextEmbedding>>>) -> Self {
        let permits = engines.len().max(1);
        Self {
            engines: Mutex::new(engines),
            semaphore: Arc::new(Semaphore::new(permits)),
        }
    }

    /// Acquire a permit and borrow a distinct idle engine. The permit guarantees an engine is
    /// available, so the pop always succeeds for a correctly sized pool.
    async fn checkout(self: &Arc<Self>) -> Result<FastEmbedLease, EmbeddingError> {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| EmbeddingError::Config("embedding pool is closed".into()))?;
        let engine = self
            .engines
            .lock()
            .map_err(EmbeddingError::mutex_poisoned)?
            .pop()
            .ok_or_else(|| EmbeddingError::Config("embedding pool unexpectedly empty".into()))?;
        Ok(FastEmbedLease {
            pool: Arc::clone(self),
            engine,
            _permit: permit,
        })
    }
}

/// Active borrow of a single `FastEmbed` engine; returns it to the pool on drop.
struct FastEmbedLease {
    pool: Arc<FastEmbedPool>,
    engine: Arc<Mutex<TextEmbedding>>,
    /// Released after the engine is returned, unblocking the next checkout.
    _permit: OwnedSemaphorePermit,
}

impl FastEmbedLease {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let engine = Arc::clone(&self.engine);
        let texts = texts.to_vec();
        tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>, EmbeddingError> {
            let mut guard = engine.lock().map_err(EmbeddingError::mutex_poisoned)?;
            guard.embed(texts, None).map_err(EmbeddingError::fastembed)
        })
        .await
        .map_err(EmbeddingError::from)?
    }
}

impl Drop for FastEmbedLease {
    fn drop(&mut self) {
        if let Ok(mut free) = self.pool.engines.lock() {
            free.push(Arc::clone(&self.engine));
        }
    }
}

async fn run_fastembed(
    pool: &Arc<FastEmbedPool>,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    let lease = pool.checkout().await?;
    lease.embed(texts).await
}

impl EmbeddingProvider {
    #[must_use]
    pub fn backend_label(&self) -> &'static str {
        match self.inner {
            EmbeddingInner::Hashed { .. } => "hashed",
            EmbeddingInner::FastEmbed { .. } => "fastembed",
            EmbeddingInner::OpenAI { .. } => "openai",
        }
    }

    #[must_use]
    pub fn dimension(&self) -> usize {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } | EmbeddingInner::FastEmbed { dimension, .. } => {
                *dimension
            }
            EmbeddingInner::OpenAI { dimensions, .. } => *dimensions as usize,
        }
    }

    #[must_use]
    pub fn model_code(&self) -> Option<String> {
        match &self.inner {
            EmbeddingInner::FastEmbed { model_name, .. } => Some(model_name.to_string()),
            EmbeddingInner::OpenAI { model, .. } => Some(model.as_ref().to_owned()),
            EmbeddingInner::Hashed { .. } => None,
        }
    }

    /// Generate an embedding vector for the given text.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError`] if the backend API call fails, FastEmbed initialisation fails,
    /// or the backend returns no embedding data.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } => Ok(hashed_embedding(text, *dimension)),
            EmbeddingInner::FastEmbed { pool, .. } => {
                let text = text.to_owned();
                let embeddings = run_fastembed(pool, std::slice::from_ref(&text)).await?;
                embeddings.into_iter().next().ok_or(EmbeddingError::NoData)
            }
            EmbeddingInner::OpenAI {
                client,
                model,
                dimensions,
            } => {
                let request = CreateEmbeddingRequestArgs::default()
                    .model(model.as_ref())
                    .input([text])
                    .dimensions(*dimensions)
                    .build()?;

                let response = client.embeddings().create(request).await?;

                let embedding = response
                    .data
                    .first()
                    .ok_or(EmbeddingError::NoData)?
                    .embedding
                    .clone();

                Ok(embedding)
            }
        }
    }

    /// Generate embedding vectors for a batch of texts.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError`] if the backend API call fails or returns no embedding data.
    /// Returns an empty `Vec` when `texts` is empty.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } => Ok(texts
                .iter()
                .map(|text| hashed_embedding(text, *dimension))
                .collect()),
            EmbeddingInner::FastEmbed { pool, .. } => {
                if texts.is_empty() {
                    return Ok(Vec::new());
                }
                run_fastembed(pool, texts).await
            }
            EmbeddingInner::OpenAI {
                client,
                model,
                dimensions,
            } => {
                if texts.is_empty() {
                    return Ok(Vec::new());
                }

                let request = CreateEmbeddingRequestArgs::default()
                    .model(model.as_ref())
                    .input(texts.to_vec())
                    .dimensions(*dimensions)
                    .build()?;

                let response = client.embeddings().create(request).await?;

                let embeddings: Vec<Vec<f32>> = response
                    .data
                    .into_iter()
                    .map(|item| item.embedding)
                    .collect();

                Ok(embeddings)
            }
        }
    }

    /// # Errors
    ///
    /// Currently infallible; reserved for future validation.
    pub fn new_openai(
        client: Arc<Client<async_openai::config::OpenAIConfig>>,
        model: impl AsRef<str>,
        dimensions: u32,
    ) -> Result<Self, EmbeddingError> {
        Ok(Self {
            inner: EmbeddingInner::OpenAI {
                client,
                model: Arc::from(model.as_ref()),
                dimensions,
            },
        })
    }

    /// Initialise a local FastEmbed provider backed by a pool of `pool_size` engines.
    ///
    /// `pool_size` is clamped to at least 1. Larger pools allow concurrent embeds at the cost of
    /// `pool_size`× model memory; see [`default_embedding_pool_size`] for guidance.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError`] if the model name is unknown or FastEmbed initialisation fails.
    pub async fn new_fastembed(
        model_override: Option<String>,
        pool_size: usize,
    ) -> Result<Self, EmbeddingError> {
        let pool_size = pool_size.max(1);
        let model_name = if let Some(code) = model_override {
            EmbeddingModel::from_str(code.trim())
                .map_err(|_| EmbeddingError::UnknownModel(unknown_fastembed_model_message(&code)))?
        } else {
            EmbeddingModel::default()
        };

        let model_name_for_task = model_name.clone();
        let model_name_code = model_name.to_string();

        let (engines, dimension) =
            match tokio::task::spawn_blocking(move || -> Result<_, EmbeddingError> {
                let info =
                    EmbeddingModel::get_model_info(&model_name_for_task).ok_or_else(|| {
                        EmbeddingError::Config(format!(
                            "fastembed model metadata missing for {model_name_code}"
                        ))
                    })?;
                let mut engines = Vec::with_capacity(pool_size);
                for index in 0..pool_size {
                    let options = TextInitOptions::new(model_name_for_task.clone())
                        // Only the first engine reports download progress; the rest reuse the cache.
                        .with_show_download_progress(index == 0);
                    let model =
                        TextEmbedding::try_new(options).map_err(EmbeddingError::fastembed)?;
                    engines.push(Arc::new(Mutex::new(model)));
                }
                Ok((engines, info.dim))
            })
            .await
            {
                Ok(result) => result?,
                Err(join_error) => return Err(EmbeddingError::from(join_error)),
            };

        Ok(EmbeddingProvider {
            inner: EmbeddingInner::FastEmbed {
                pool: Arc::new(FastEmbedPool::new(engines)),
                model_name,
                dimension,
            },
        })
    }

    /// # Errors
    ///
    /// Currently infallible; reserved for future validation.
    pub fn new_hashed(dimension: usize) -> Result<Self, EmbeddingError> {
        Ok(EmbeddingProvider {
            inner: EmbeddingInner::Hashed {
                dimension: dimension.max(1),
            },
        })
    }

    /// Creates an embedding provider from persisted settings and bootstrap config.
    ///
    /// OpenAI/hashed model settings come from [`SystemSettings`]. FastEmbed uses
    /// [`resolve_fastembed_model_code`] (config `fastembed_model` overrides DB). The active
    /// backend is taken from `config.embedding_backend`; [`SystemSettings::sync_from_embedding_provider`]
    /// persists the resolved backend to the database after startup.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError`] if the selected backend cannot be initialised.
    pub async fn from_system_settings(
        settings: &SystemSettings,
        config: &AppConfig,
        openai_client: Option<Arc<Client<async_openai::config::OpenAIConfig>>>,
    ) -> Result<Self, EmbeddingError> {
        let dimensions = settings.embedding_dimensions;
        match config.embedding_backend {
            EmbeddingBackend::OpenAI => {
                let client = openai_client.ok_or_else(|| {
                    EmbeddingError::Config(
                        "openai embedding backend requires an openai client".into(),
                    )
                })?;
                Self::new_openai(client, settings.embedding_model.as_str(), dimensions)
            }
            EmbeddingBackend::FastEmbed => {
                let pool_size = config
                    .embedding_pool_size
                    .unwrap_or_else(default_embedding_pool_size);
                let model_code = resolve_fastembed_model_code(config, &settings.embedding_model)?;
                Self::new_fastembed(Some(model_code), pool_size).await
            }
            EmbeddingBackend::Hashed => {
                let dimension = usize::try_from(dimensions).map_err(|_| {
                    EmbeddingError::Config("embedding_dimensions exceeds usize::MAX".into())
                })?;
                Self::new_hashed(dimension)
            }
        }
    }
}

// Helper functions for hashed embeddings
/// Generates a hashed embedding vector without external dependencies.
fn hashed_embedding(text: &str, dimension: usize) -> Vec<f32> {
    let dim = dimension.max(1);
    let mut vector = vec![0.0f32; dim];
    if text.is_empty() {
        return vector;
    }

    for token in tokens(text) {
        let idx = bucket(&token, dim);
        if let Some(slot) = vector.get_mut(idx) {
            *slot += 1.0;
        }
    }

    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }

    vector
}

/// Tokenizes the text into alphanumeric lowercase tokens.
fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Buckets a token into the hashed embedding vector.
#[allow(clippy::arithmetic_side_effects)]
fn bucket(token: &str, dimension: usize) -> usize {
    let safe_dimension = dimension.max(1);
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    usize::try_from(hasher.finish()).unwrap_or_default() % safe_dimension
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{
        align_fastembed_system_settings, fastembed_model_dimension,
        list_fastembed_embedding_models, resolve_fastembed_model_code, EmbeddingError,
        DEFAULT_FASTEMBED_MODEL_CODE,
    };
    use crate::storage::types::system_settings::SystemSettings;
    use crate::utils::config::{AppConfig, EmbeddingBackend, ParseEmbeddingBackendError};
    use serde_json::json;

    #[test]
    fn embedding_backend_defaults_to_fastembed() {
        assert_eq!(EmbeddingBackend::default(), EmbeddingBackend::FastEmbed);
    }

    #[test]
    fn embedding_backend_as_str_matches_serde_names() {
        assert_eq!(EmbeddingBackend::OpenAI.as_str(), "openai");
        assert_eq!(EmbeddingBackend::FastEmbed.as_str(), "fastembed");
        assert_eq!(EmbeddingBackend::Hashed.as_str(), "hashed");

        assert_eq!(
            serde_json::to_string(&EmbeddingBackend::FastEmbed).expect("serialize"),
            "\"fastembed\""
        );
    }

    #[test]
    fn embedding_backend_deserializes_lowercase_values() {
        let openai: EmbeddingBackend = serde_json::from_str("\"openai\"").expect("openai");
        let fastembed: EmbeddingBackend = serde_json::from_str("\"fastembed\"").expect("fastembed");
        let hashed: EmbeddingBackend = serde_json::from_str("\"hashed\"").expect("hashed");

        assert_eq!(openai, EmbeddingBackend::OpenAI);
        assert_eq!(fastembed, EmbeddingBackend::FastEmbed);
        assert_eq!(hashed, EmbeddingBackend::Hashed);
    }

    #[test]
    fn embedding_backend_from_str_accepts_aliases() {
        assert_eq!(
            "fast-embed"
                .parse::<EmbeddingBackend>()
                .expect("fast-embed"),
            EmbeddingBackend::FastEmbed
        );
        assert_eq!(
            "FASTEMBED".parse::<EmbeddingBackend>().expect("FASTEMBED"),
            EmbeddingBackend::FastEmbed
        );
        assert!(matches!(
            "unknown-backend".parse::<EmbeddingBackend>(),
            Err(ParseEmbeddingBackendError { .. })
        ));
    }

    #[test]
    fn list_fastembed_embedding_models_includes_default() {
        let models = list_fastembed_embedding_models();
        assert!(
            models
                .iter()
                .any(|m| m.model_code == DEFAULT_FASTEMBED_MODEL_CODE),
            "catalog should include the default FastEmbed model"
        );
    }

    #[test]
    fn resolve_fastembed_model_prefers_config_over_db() {
        let config = AppConfig {
            fastembed_model: Some("Xenova/bge-base-en-v1.5".into()),
            ..AppConfig::default()
        };
        let resolved =
            resolve_fastembed_model_code(&config, "text-embedding-3-small").expect("config model");
        assert_eq!(resolved, "Xenova/bge-base-en-v1.5");
    }

    #[test]
    fn resolve_fastembed_model_falls_back_from_openai_default() {
        let config = AppConfig::default();
        let resolved =
            resolve_fastembed_model_code(&config, "text-embedding-3-small").expect("default model");
        assert_eq!(resolved, DEFAULT_FASTEMBED_MODEL_CODE);
    }

    #[test]
    fn resolve_fastembed_model_rejects_invalid_config_override() {
        let config = AppConfig {
            fastembed_model: Some("not-a-real-model".into()),
            ..AppConfig::default()
        };
        let err = resolve_fastembed_model_code(&config, "Xenova/bge-small-en-v1.5")
            .expect_err("invalid config model");
        assert!(matches!(err, EmbeddingError::UnknownModel(_)));
    }

    #[test]
    fn fastembed_model_dimension_matches_model_metadata() {
        let dim = fastembed_model_dimension(DEFAULT_FASTEMBED_MODEL_CODE).expect("dim");
        assert_eq!(dim, 384);
    }

    #[tokio::test]
    async fn align_fastembed_system_settings_replaces_openai_default() -> anyhow::Result<()> {
        use crate::storage::db::SurrealDbClient;
        use uuid::Uuid;

        let db = SurrealDbClient::memory("align_fe", &Uuid::new_v4().to_string()).await?;
        db.apply_migrations().await?;

        let config = AppConfig {
            embedding_backend: EmbeddingBackend::FastEmbed,
            ..AppConfig::default()
        };

        let settings = align_fastembed_system_settings(&db, &config).await?;
        assert_eq!(settings.embedding_model, DEFAULT_FASTEMBED_MODEL_CODE);
        assert_eq!(settings.embedding_dimensions, 384);
        Ok(())
    }

    #[test]
    fn system_settings_deserializes_embedding_backend_field() {
        let value = json!({
            "id": "current",
            "registrations_enabled": true,
            "require_email_verification": false,
            "query_model": "gpt-4o-mini",
            "processing_model": "gpt-4o-mini",
            "embedding_model": "text-embedding-3-small",
            "embedding_dimensions": 1536,
            "embedding_backend": "hashed",
            "query_system_prompt": "query",
            "ingestion_system_prompt": "ingestion",
            "image_processing_model": "gpt-4o-mini",
            "image_processing_prompt": "image",
            "voice_processing_model": "whisper-1",
        });

        let settings: SystemSettings =
            serde_json::from_value(value).expect("deserialize system settings");
        assert_eq!(settings.embedding_backend, Some(EmbeddingBackend::Hashed));
    }
}
