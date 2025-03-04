use std::sync::Arc;

use common::{ingress::jobqueue::JobQueue, storage::db::SurrealDbClient, utils::config::AppConfig};

#[derive(Clone)]
pub struct ApiState {
    pub surreal_db_client: Arc<SurrealDbClient>,
    pub job_queue: Arc<JobQueue>,
}

impl ApiState {
    pub async fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
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

        let app_state = ApiState {
            surreal_db_client: surreal_db_client.clone(),
            job_queue: Arc::new(JobQueue::new(surreal_db_client)),
        };

        Ok(app_state)
    }
}
