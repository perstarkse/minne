use crate::ingress::jobqueue::JobQueue;
use crate::storage::db::SurrealDbClient;
use crate::utils::config::AppConfig;
use crate::utils::mailer::Mailer;
use axum_session::SessionStore;
use axum_session_surreal::SessionSurrealPool;
use minijinja::{path_loader, Environment};
use minijinja_autoreload::AutoReloader;
use std::path::PathBuf;
use std::sync::Arc;
use surrealdb::engine::any::Any;

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
    pub session_store: Arc<SessionStore<SessionSurrealPool<Any>>>,
}

impl AppState {
    pub async fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let reloader = AutoReloader::new(move |notifier| {
            let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
            let mut env = Environment::new();
            env.set_loader(path_loader(&template_path));

            notifier.set_fast_reload(true);
            notifier.watch_path(&template_path, true);
            minijinja_contrib::add_to_environment(&mut env);
            Ok(env)
        });

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

        let app_state = AppState {
            surreal_db_client: surreal_db_client.clone(),
            templates: Arc::new(reloader),
            openai_client: openai_client.clone(),
            mailer: Arc::new(Mailer::new(
                &config.smtp_username,
                &config.smtp_relayer,
                &config.smtp_password,
            )?),
            job_queue: Arc::new(JobQueue::new(surreal_db_client, openai_client)),
            session_store,
        };

        Ok(app_state)
    }
}
