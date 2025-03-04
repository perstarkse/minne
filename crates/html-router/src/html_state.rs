use axum_session::SessionStore;
use axum_session_surreal::SessionSurrealPool;
use common::ingress::jobqueue::JobQueue;
use common::storage::db::SurrealDbClient;
use common::utils::config::AppConfig;
use common::utils::mailer::Mailer;
use minijinja::{path_loader, Environment};
use minijinja_autoreload::AutoReloader;
use std::path::PathBuf;
use std::sync::Arc;
use surrealdb::engine::any::Any;

#[derive(Clone)]
pub struct HtmlState {
    pub surreal_db_client: Arc<SurrealDbClient>,
    pub openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    pub templates: Arc<AutoReloader>,
    pub mailer: Arc<Mailer>,
    pub job_queue: Arc<JobQueue>,
    pub session_store: Arc<SessionStore<SessionSurrealPool<Any>>>,
}

impl HtmlState {
    pub async fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let reloader = AutoReloader::new(move |notifier| {
            let template_path = get_templates_dir();
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

        let app_state = HtmlState {
            surreal_db_client: surreal_db_client.clone(),
            templates: Arc::new(reloader),
            openai_client: openai_client.clone(),
            mailer: Arc::new(Mailer::new(
                &config.smtp_username,
                &config.smtp_relayer,
                &config.smtp_password,
            )?),
            job_queue: Arc::new(JobQueue::new(surreal_db_client)),
            session_store,
        };

        Ok(app_state)
    }
}

pub fn get_workspace_root() -> PathBuf {
    // Starts from CARGO_MANIFEST_DIR (e.g., /project/crates/html-router/)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Navigate up to /path/to/project/crates
    let crates_dir = manifest_dir
        .parent()
        .expect("Failed to find parent of manifest directory");

    // Navigate up to workspace root
    crates_dir
        .parent()
        .expect("Failed to find workspace root")
        .to_path_buf()
}

pub fn get_templates_dir() -> PathBuf {
    get_workspace_root().join("templates")
}
