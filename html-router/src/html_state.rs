use common::storage::types::conversation::SidebarConversation;
use common::storage::{db::SurrealDbClient, store::StorageManager};
use common::utils::embedding::EmbeddingProvider;
use common::utils::template_engine::{ProvidesTemplateEngine, TemplateEngine};
use common::{create_template_engine, storage::db::ProvidesDb, utils::config::AppConfig};
use retrieval_pipeline::{reranking::RerankerPool, RetrievalStrategy};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
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
    conversation_archive_cache_writes: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct ConversationArchiveCacheEntry {
    conversations: Vec<SidebarConversation>,
    expires_at: Instant,
}

const CONVERSATION_ARCHIVE_CACHE_TTL: Duration = Duration::from_secs(30);
const CONVERSATION_ARCHIVE_CACHE_MAX_USERS: usize = 1024;
const CONVERSATION_ARCHIVE_CACHE_CLEANUP_WRITE_INTERVAL: usize = 64;

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
            conversation_archive_cache_writes: Arc::new(AtomicUsize::new(0)),
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
    ) -> Option<Vec<SidebarConversation>> {
        let now = Instant::now();
        let should_evict_expired = {
            let cache = self.conversation_archive_cache.read().await;
            if let Some(entry) = cache.get(user_id) {
                if entry.expires_at > now {
                    return Some(entry.conversations.clone());
                }
                true
            } else {
                false
            }
        };

        if should_evict_expired {
            let mut cache = self.conversation_archive_cache.write().await;
            cache.remove(user_id);
        }

        None
    }

    pub async fn set_cached_conversation_archive(
        &self,
        user_id: &str,
        conversations: Vec<SidebarConversation>,
    ) {
        let now = Instant::now();
        let mut cache = self.conversation_archive_cache.write().await;
        cache.insert(
            user_id.to_string(),
            ConversationArchiveCacheEntry {
                conversations,
                expires_at: now + CONVERSATION_ARCHIVE_CACHE_TTL,
            },
        );

        let writes = self
            .conversation_archive_cache_writes
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if writes % CONVERSATION_ARCHIVE_CACHE_CLEANUP_WRITE_INTERVAL == 0 {
            Self::purge_expired_entries(&mut cache, now);
        }

        Self::enforce_cache_capacity(&mut cache);
    }

    pub async fn invalidate_conversation_archive_cache(&self, user_id: &str) {
        let mut cache = self.conversation_archive_cache.write().await;
        cache.remove(user_id);
    }

    fn purge_expired_entries(
        cache: &mut HashMap<String, ConversationArchiveCacheEntry>,
        now: Instant,
    ) {
        cache.retain(|_, entry| entry.expires_at > now);
    }

    fn enforce_cache_capacity(cache: &mut HashMap<String, ConversationArchiveCacheEntry>) {
        if cache.len() <= CONVERSATION_ARCHIVE_CACHE_MAX_USERS {
            return;
        }

        let overflow = cache.len() - CONVERSATION_ARCHIVE_CACHE_MAX_USERS;
        let mut by_expiry: Vec<(String, Instant)> = cache
            .iter()
            .map(|(user_id, entry)| (user_id.clone(), entry.expires_at))
            .collect();
        by_expiry.sort_by_key(|(_, expires_at)| *expires_at);

        for (user_id, _) in by_expiry.into_iter().take(overflow) {
            cache.remove(&user_id);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use common::{
        storage::types::conversation::SidebarConversation,
        utils::{
            config::{AppConfig, StorageKind},
            embedding::EmbeddingProvider,
        },
    };

    async fn test_state() -> HtmlState {
        let namespace = "test_ns";
        let database = &uuid::Uuid::new_v4().to_string();
        let db = Arc::new(
            SurrealDbClient::memory(namespace, database)
                .await
                .expect("Failed to create in-memory DB"),
        );

        let session_store = Arc::new(
            db.create_session_store()
                .await
                .expect("Failed to create session store"),
        );

        let mut config = AppConfig::default();
        config.storage = StorageKind::Memory;

        let storage = StorageManager::new(&config)
            .await
            .expect("Failed to create storage manager");

        let embedding_provider = Arc::new(
            EmbeddingProvider::new_hashed(8).expect("Failed to create embedding provider"),
        );

        HtmlState::new_with_resources(
            db,
            Arc::new(async_openai::Client::new()),
            session_store,
            storage,
            config,
            None,
            embedding_provider,
            None,
        )
        .await
        .expect("Failed to create HtmlState")
    }

    #[tokio::test]
    async fn test_expired_conversation_archive_entry_is_evicted_on_read() {
        let state = test_state().await;
        let user_id = "expired-user";

        {
            let mut cache = state.conversation_archive_cache.write().await;
            cache.insert(
                user_id.to_string(),
                ConversationArchiveCacheEntry {
                    conversations: vec![SidebarConversation {
                        id: "conv-1".to_string(),
                        title: "A stale chat".to_string(),
                    }],
                    expires_at: Instant::now() - Duration::from_secs(1),
                },
            );
        }

        let cached = state.get_cached_conversation_archive(user_id).await;
        assert!(
            cached.is_none(),
            "Expired cache entry should not be returned"
        );

        let cache = state.conversation_archive_cache.read().await;
        assert!(
            !cache.contains_key(user_id),
            "Expired cache entry should be evicted after read"
        );
    }
}
