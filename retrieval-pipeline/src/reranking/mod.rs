use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::available_parallelism,
};

use common::{error::AppError, utils::config::AppConfig};
use fastembed::{RerankInitOptions, RerankResult, TextRerank};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tracing::debug;

static NEXT_ENGINE: AtomicUsize = AtomicUsize::new(0);

fn pick_engine_index(pool_len: usize) -> usize {
    let n = NEXT_ENGINE.fetch_add(1, Ordering::Relaxed);
    n % pool_len
}

pub struct RerankerPool {
    engines: Vec<Arc<Mutex<TextRerank>>>,
    semaphore: Arc<Semaphore>,
}

impl RerankerPool {
    /// Build the pool at startup.
    /// `pool_size` controls max parallel reranks.
    pub fn new(pool_size: usize) -> Result<Arc<Self>, AppError> {
        Self::new_with_options(pool_size, RerankInitOptions::default())
    }

    fn new_with_options(
        pool_size: usize,
        init_options: RerankInitOptions,
    ) -> Result<Arc<Self>, AppError> {
        if pool_size == 0 {
            return Err(AppError::Validation(
                "RERANKING_POOL_SIZE must be greater than zero".to_string(),
            ));
        }

        fs::create_dir_all(&init_options.cache_dir)?;

        let mut engines = Vec::with_capacity(pool_size);
        for x in 0..pool_size {
            debug!("Creating reranking engine: {x}");
            let model = TextRerank::try_new(init_options.clone())
                .map_err(|e| AppError::InternalError(e.to_string()))?;
            engines.push(Arc::new(Mutex::new(model)));
        }

        Ok(Arc::new(Self {
            engines,
            semaphore: Arc::new(Semaphore::new(pool_size)),
        }))
    }

    /// Initialize a pool using application configuration.
    pub fn maybe_from_config(config: &AppConfig) -> Result<Option<Arc<Self>>, AppError> {
        if !config.reranking_enabled {
            return Ok(None);
        }

        let pool_size = config.reranking_pool_size.unwrap_or_else(default_pool_size);

        let init_options = build_rerank_init_options(config)?;
        Self::new_with_options(pool_size, init_options).map(Some)
    }

    /// Check out capacity + pick an engine.
    /// This returns a lease that can perform rerank().
    pub async fn checkout(self: &Arc<Self>) -> RerankerLease {
        // Acquire a permit. This enforces backpressure.
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore closed");

        // Pick an engine.
        // This is naive: just pick based on a simple modulo counter.
        // We use an atomic counter to avoid always choosing index 0.
        let idx = pick_engine_index(self.engines.len());
        let engine = self.engines[idx].clone();

        RerankerLease {
            _permit: permit,
            engine,
        }
    }
}

fn default_pool_size() -> usize {
    available_parallelism()
        .map(|value| value.get().min(2))
        .unwrap_or(2)
        .max(1)
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn build_rerank_init_options(config: &AppConfig) -> Result<RerankInitOptions, AppError> {
    let mut options = RerankInitOptions::default();

    let cache_dir = config
        .fastembed_cache_dir
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| env::var("RERANKING_CACHE_DIR").ok().map(PathBuf::from))
        .or_else(|| env::var("FASTEMBED_CACHE_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            Path::new(&config.data_dir)
                .join("fastembed")
                .join("reranker")
        });
    fs::create_dir_all(&cache_dir)?;
    options.cache_dir = cache_dir;

    let show_progress = config
        .fastembed_show_download_progress
        .or_else(|| env_bool("RERANKING_SHOW_DOWNLOAD_PROGRESS"))
        .or_else(|| env_bool("FASTEMBED_SHOW_DOWNLOAD_PROGRESS"))
        .unwrap_or(true);
    options.show_download_progress = show_progress;

    if let Some(max_length) = config.fastembed_max_length.or_else(|| {
        env::var("RERANKING_MAX_LENGTH")
            .ok()
            .and_then(|value| value.parse().ok())
    }) {
        options.max_length = max_length;
    }

    Ok(options)
}

fn env_bool(key: &str) -> Option<bool> {
    env::var(key).ok().map(|value| is_truthy(&value))
}

/// Active lease on a single TextRerank instance.
pub struct RerankerLease {
    // When this drops the semaphore permit is released.
    _permit: OwnedSemaphorePermit,
    engine: Arc<Mutex<TextRerank>>,
}

impl RerankerLease {
    pub async fn rerank(
        &self,
        query: &str,
        documents: Vec<String>,
    ) -> Result<Vec<RerankResult>, AppError> {
        // Lock this specific engine so we get &mut TextRerank
        let mut guard = self.engine.lock().await;

        guard
            .rerank(query.to_owned(), documents, false, None)
            .map_err(|e| AppError::InternalError(e.to_string()))
    }
}
