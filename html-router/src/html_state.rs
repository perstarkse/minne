use common::storage::{db::SurrealDbClient, store::StorageManager};
use common::utils::embedding::EmbeddingProvider;
use common::utils::template_engine::{ProvidesTemplateEngine, TemplateEngine};
use common::{create_template_engine, storage::db::ProvidesDb, utils::config::AppConfig};
use retrieval_pipeline::{reranking::RerankerPool, RetrievalStrategy};
use std::sync::Arc;
use tracing::debug;

use crate::{OpenAIClientType, SessionStoreType};

#[derive(Clone)]
pub struct HtmlState {
    pub db: Arc<SurrealDbClient>,
    pub openai_client: Arc<OpenAIClientType>,
    pub templates: Arc<TemplateEngine>,
    pub session_store: Arc<SessionStoreType>,
    pub config: AppConfig,
    pub storage: StorageManager,
    pub reranker_pool: Option<Arc<RerankerPool>>,
    pub embedding_provider: Arc<EmbeddingProvider>,
}

impl HtmlState {
    pub async fn new_with_resources(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<OpenAIClientType>,
        session_store: Arc<SessionStoreType>,
        storage: StorageManager,
        config: AppConfig,
        reranker_pool: Option<Arc<RerankerPool>>,
        embedding_provider: Arc<EmbeddingProvider>,
        template_engine: Option<Arc<TemplateEngine>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let templates =
            template_engine.unwrap_or_else(|| Arc::new(create_template_engine!("templates")));
        debug!("Template engine configured for html_router.");

        Ok(Self {
            db,
            openai_client,
            session_store,
            templates,
            config,
            storage,
            reranker_pool,
            embedding_provider,
        })
    }

    pub fn retrieval_strategy(&self) -> RetrievalStrategy {
        self.config
            .retrieval_strategy
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(RetrievalStrategy::Default)
    }
}
impl ProvidesDb for HtmlState {
    fn db(&self) -> &Arc<SurrealDbClient> {
        &self.db
    }
}
impl ProvidesTemplateEngine for HtmlState {
    fn template_engine(&self) -> &Arc<TemplateEngine> {
        &self.templates
    }
}

impl crate::middlewares::response_middleware::ProvidesHtmlState for HtmlState {
    fn html_state(&self) -> &HtmlState {
        self
    }
}
