use std::sync::Arc;

use async_openai::Client;
use common::{
    storage::{
        db::SurrealDbClient,
        store::StorageManager,
    },
    utils::{
        config::{get_config, AppConfig},
        embedding::EmbeddingProvider,
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
        .await?,
    );

    db.apply_migrations().await?;

    let openai_client = Arc::new(Client::with_config(
        async_openai::config::OpenAIConfig::new()
            .with_api_key(&config.openai_api_key)
            .with_api_base(&config.openai_base_url),
    ));

    let embedding_provider = Arc::new(
        EmbeddingProvider::from_config(&config, Some(Arc::clone(&openai_client))).await?,
    );

    let reranker_pool = RerankerPool::maybe_from_config(&config)?;

    let storage = StorageManager::new(&config).await?;

    Ok(SharedServices {
        db,
        openai_client,
        embedding_provider,
        storage,
        reranker_pool,
        config,
    })
}
