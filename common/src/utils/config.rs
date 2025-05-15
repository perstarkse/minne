use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

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
}

fn default_data_dir() -> String {
    "./data".to_string()
}

pub fn get_config() -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        .add_source(File::with_name("config").required(false))
        .add_source(Environment::default())
        .build()?;

    Ok(config.try_deserialize()?)
}
