use std::sync::Arc;

use common::{
    storage::{db::SurrealDbClient, store::StorageManager},
    utils::config::AppConfig,
};

#[derive(Clone)]
pub struct ApiState {
    pub db: Arc<SurrealDbClient>,
    pub config: AppConfig,
    pub storage: StorageManager,
}

impl ApiState {
    pub async fn new(
        config: &AppConfig,
        storage: StorageManager,
    ) -> Result<Self, Box<dyn std::error::Error>> {
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

        surreal_db_client.apply_migrations().await?;

        let app_state = Self {
            db: surreal_db_client.clone(),
            config: config.clone(),
            storage,
        };

        Ok(app_state)
    }
}
