use common::storage::db::SurrealDbClient;
use common::utils::template_engine::{ProvidesTemplateEngine, TemplateEngine};
use common::{create_template_engine, storage::db::ProvidesDb};
use std::sync::Arc;
use tracing::debug;

use crate::{OpenAIClientType, SessionStoreType};

#[derive(Clone)]
pub struct HtmlState {
    pub db: Arc<SurrealDbClient>,
    pub openai_client: Arc<OpenAIClientType>,
    pub templates: Arc<TemplateEngine>,
    pub session_store: Arc<SessionStoreType>,
}

impl HtmlState {
    pub fn new_with_resources(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<OpenAIClientType>,
        session_store: Arc<SessionStoreType>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let template_engine = create_template_engine!("templates");
        debug!("Template engine created for html_router.");

        Ok(Self {
            db,
            openai_client,
            session_store,
            templates: Arc::new(template_engine),
        })
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
