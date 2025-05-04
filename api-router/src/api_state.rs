use std::sync::Arc;

use common::{storage::db::SurrealDbClient, utils::config::AppConfig};

#[derive(Clone)]
pub struct ApiState {
    pub db: Arc<SurrealDbClient>,
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

        surreal_db_client.apply_migrations().await?;

        let app_state = ApiState {
            db: surreal_db_client.clone(),
        };

        Ok(app_state)
    }
}
