mod startup;
pub mod wiring;

pub use startup::{prepare_embedding_runtime, EmbeddingRuntimeRole};

use std::sync::Arc;

use anyhow::Context;
use async_openai::Client;
use common::{
    storage::{
        db::SurrealDbClient,
        store::StorageManager,
    },
    utils::{
        config::{get_config, AppConfig},
        embedding::{align_fastembed_system_settings, EmbeddingProvider},
    },
};
use retrieval_pipeline::reranking::RerankerPool;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub struct SharedServices {
    pub db: Arc<SurrealDbClient>,
    pub openai_client: Arc<Client<async_openai::config::OpenAIConfig>>,
    pub embedding_provider: Arc<EmbeddingProvider>,
    pub storage: StorageManager,
    pub reranker_pool: Option<Arc<RerankerPool>>,
    pub config: AppConfig,
}

pub async fn init() -> anyhow::Result<SharedServices> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;
    init_with_config(config).await
}

pub(crate) async fn init_with_config(config: AppConfig) -> anyhow::Result<SharedServices> {
    let db = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await
        .context("connect to surrealdb")?,
    );

    db.apply_migrations()
        .await
        .context("apply database migrations")?;

    let settings = align_fastembed_system_settings(&db, &config)
        .await
        .context("align fastembed system settings")?;

    let openai_client = Arc::new(Client::with_config(
        async_openai::config::OpenAIConfig::new()
            .with_api_key(&config.openai_api_key)
            .with_api_base(&config.openai_base_url),
    ));

    let embedding_provider = Arc::new(
        EmbeddingProvider::from_system_settings(
            &settings,
            &config,
            Some(Arc::clone(&openai_client)),
        )
        .await
        .context("initialize embedding provider")?,
    );

    let reranker_pool = RerankerPool::maybe_from_config(&config)?;

    let storage = StorageManager::new(&config)
        .await
        .context("initialize storage manager")?;

    Ok(SharedServices {
        db,
        openai_client,
        embedding_provider,
        storage,
        reranker_pool,
        config,
    })
}

#[cfg(test)]
#[allow(dead_code)] // helpers are shared across binary test targets
pub(crate) mod tests {
    use std::path::Path;

    use anyhow::Context;
    use common::utils::config::{AppConfig, EmbeddingBackend, PdfIngestMode, StorageKind};
    use uuid::Uuid;

    pub fn smoke_test_config(namespace: &str, database: &str, data_dir: &Path) -> AppConfig {
        AppConfig {
            openai_api_key: "test-key".into(),
            surrealdb_address: "mem://".into(),
            surrealdb_username: "root".into(),
            surrealdb_password: "root".into(),
            surrealdb_namespace: namespace.into(),
            surrealdb_database: database.into(),
            data_dir: data_dir.to_string_lossy().into_owned(),
            http_port: 0,
            openai_base_url: "https://example.com".into(),
            storage: StorageKind::Local,
            pdf_ingest_mode: PdfIngestMode::LlmFirst,
            embedding_backend: EmbeddingBackend::Hashed,
            ..Default::default()
        }
    }

    pub async fn init_smoke_services() -> anyhow::Result<(super::SharedServices, std::path::PathBuf)>
    {
        let namespace = "test_ns";
        let database = format!("test_db_{}", Uuid::new_v4());
        let data_dir = std::env::temp_dir().join(format!("minne_smoke_{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&data_dir)
            .await
            .context("create temp data directory")?;

        let config = smoke_test_config(namespace, &database, &data_dir);
        let services = super::init_with_config(config).await?;
        Ok((services, data_dir))
    }
}
