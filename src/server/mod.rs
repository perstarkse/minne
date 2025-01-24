use crate::ingress::jobqueue::JobQueue;
use crate::storage::db::SurrealDbClient;
use crate::utils::mailer::Mailer;
use minijinja_autoreload::AutoReloader;
use std::sync::Arc;

pub mod middleware_analytics;
pub mod middleware_api_auth;
pub mod routes;

#[derive(Clone)]
pub struct AppState {
    pub surreal_db_client: Arc<SurrealDbClient>,
    pub openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    pub templates: Arc<AutoReloader>,
    pub mailer: Arc<Mailer>,
    pub job_queue: Arc<JobQueue>,
}
