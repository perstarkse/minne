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
