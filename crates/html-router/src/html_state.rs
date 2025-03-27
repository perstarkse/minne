use axum_session::SessionStore;
use axum_session_surreal::SessionSurrealPool;
use common::create_template_engine;
use common::storage::db::SurrealDbClient;
use common::utils::config::AppConfig;
use common::utils::template_engine::TemplateEngine;
use std::sync::Arc;
use surrealdb::engine::any::Any;

#[derive(Clone)]
pub struct HtmlState {
    pub db: Arc<SurrealDbClient>,
    pub openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    pub templates: Arc<TemplateEngine>,
    pub session_store: Arc<SessionStore<SessionSurrealPool<Any>>>,
}

impl HtmlState {
    pub async fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let template_engine = create_template_engine!("templates");

        let surreal_db_client = Arc::new(
            SurrealDbClient::new(
                &config.surrealdb_address,
                &config.surrealdb_username,
                &config.surrealdb_password,
                &config.surrealdb_namespace,
                &config.surrealdb_database,
            )
            .await?,
        );

        surreal_db_client.ensure_initialized().await?;

        let openai_client = Arc::new(async_openai::Client::new());

        let session_store = Arc::new(surreal_db_client.create_session_store().await?);

        let app_state = HtmlState {
            db: surreal_db_client.clone(),
            templates: Arc::new(template_engine),
            openai_client: openai_client.clone(),
            session_store,
        };

        Ok(app_state)
    }
}
