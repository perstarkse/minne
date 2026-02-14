use common::storage::types::conversation::Conversation;
use common::storage::{db::SurrealDbClient, store::StorageManager};
use common::utils::embedding::EmbeddingProvider;
use common::utils::template_engine::{ProvidesTemplateEngine, TemplateEngine};
use common::{create_template_engine, storage::db::ProvidesDb, utils::config::AppConfig};
use retrieval_pipeline::{reranking::RerankerPool, RetrievalStrategy};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
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
    conversation_archive_cache: Arc<RwLock<HashMap<String, ConversationArchiveCacheEntry>>>,
}

#[derive(Clone)]
struct ConversationArchiveCacheEntry {
    conversations: Vec<Conversation>,
    expires_at: Instant,
}

const CONVERSATION_ARCHIVE_CACHE_TTL: Duration = Duration::from_secs(30);

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
            conversation_archive_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn retrieval_strategy(&self) -> RetrievalStrategy {
        self.config
            .retrieval_strategy
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(RetrievalStrategy::Default)
    }

    pub async fn get_cached_conversation_archive(
        &self,
        user_id: &str,
    ) -> Option<Vec<Conversation>> {
        let cache = self.conversation_archive_cache.read().await;
        let entry = cache.get(user_id)?;
        if entry.expires_at <= Instant::now() {
            return None;
        }
        Some(entry.conversations.clone())
    }

    pub async fn set_cached_conversation_archive(
        &self,
        user_id: &str,
        conversations: Vec<Conversation>,
    ) {
        let mut cache = self.conversation_archive_cache.write().await;
        cache.insert(
            user_id.to_string(),
            ConversationArchiveCacheEntry {
                conversations,
                expires_at: Instant::now() + CONVERSATION_ARCHIVE_CACHE_TTL,
            },
        );
    }

    pub async fn invalidate_conversation_archive_cache(&self, user_id: &str) {
        let mut cache = self.conversation_archive_cache.write().await;
        cache.remove(user_id);
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
