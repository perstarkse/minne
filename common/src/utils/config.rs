use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum StorageKind {
    Local,
}

fn default_storage_kind() -> StorageKind {
    StorageKind::Local
}

#[derive(Clone, Deserialize, Debug)]
pub struct AppConfig {
    pub openai_api_key: String,
    pub surrealdb_address: String,
    pub surrealdb_username: String,
    pub surrealdb_password: String,
    pub surrealdb_namespace: String,
    pub surrealdb_database: String,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    pub http_port: u16,
    #[serde(default = "default_base_url")]
    pub openai_base_url: String,
    #[serde(default = "default_storage_kind")]
    pub storage: StorageKind,
}

fn default_data_dir() -> String {
    "./data".to_string()
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

pub fn get_config() -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        .add_source(File::with_name("config").required(false))
        .add_source(Environment::default())
        .build()?;

    config.try_deserialize()
}
