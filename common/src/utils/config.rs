use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use crate::storage::backends::{StorageConfig, create_storage_backend, StorageBackend, StorageError};

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
    
    // Storage backend configuration
    #[serde(default = "default_storage_backend")]
    pub storage_backend: String,
    
    // S3 configuration
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub s3_prefix: Option<String>,
}

fn default_data_dir() -> String {
    "./data".to_string()
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_storage_backend() -> String {
    "filesystem".to_string()
}

pub fn get_config() -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        .add_source(File::with_name("config").required(false))
        .add_source(Environment::default())
        .build()?;

    Ok(config.try_deserialize()?)
}

impl AppConfig {
    /// Create a storage backend based on the configuration
    pub async fn create_storage_backend(&self) -> Result<Box<dyn StorageBackend>, StorageError> {
        let storage_config = match self.storage_backend.to_lowercase().as_str() {
            "filesystem" | "local" => {
                StorageConfig::FileSystem {
                    data_dir: self.data_dir.clone(),
                }
            }
            "s3" => {
                let bucket = self.s3_bucket.as_ref()
                    .ok_or_else(|| StorageError::Config("S3_BUCKET is required for S3 backend".to_string()))?
                    .clone();
                
                StorageConfig::S3 {
                    bucket,
                    region: self.s3_region.clone(),
                    endpoint: self.s3_endpoint.clone(),
                    access_key_id: self.s3_access_key_id.clone(),
                    secret_access_key: self.s3_secret_access_key.clone(),
                    prefix: self.s3_prefix.clone(),
                }
            }
            backend => {
                return Err(StorageError::Config(format!(
                    "Unsupported storage backend: {}. Supported backends: filesystem, s3", 
                    backend
                )));
            }
        };

        create_storage_backend(storage_config).await
    }
}
