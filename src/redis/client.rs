use redis::AsyncCommands;
use thiserror::Error;
use uuid::Uuid;

use crate::models::file_info::FileInfo;

/// Represents errors that can occur during Redis operations.
#[derive(Error, Debug)]
pub enum RedisError {
    #[error("Redis connection error: {0}")]
    ConnectionError(String),

    #[error("Redis command error: {0}")]
    CommandError(String),

    // Add more error variants as needed.
}

/// Provides Redis-related operations for `FileInfo`.
pub struct RedisClient {
    redis_url: String,
}

impl RedisClient {
    /// Creates a new instance of `RedisClient`.
    ///
    /// # Arguments
    ///
    /// * `redis_url` - The Redis server URL (e.g., "redis://127.0.0.1/").
    ///
    /// # Returns
    ///
    /// * `Self` - A new `RedisClient` instance.
    pub fn new(redis_url: &str) -> Self {
        Self {
            redis_url: redis_url.to_string(),
        }
    }

    /// Establishes a new multiplexed asynchronous connection to Redis.
    ///
    /// # Returns
    ///
    /// * `MultiplexedConnection` - The established connection.
    pub async fn get_connection(&self) -> Result<redis::aio::MultiplexedConnection, RedisError> {
        let client = redis::Client::open(self.redis_url.clone())
            .map_err(|e| RedisError::ConnectionError(e.to_string()))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| RedisError::ConnectionError(e.to_string()))?;
        Ok(conn)
    }

    /// Stores `FileInfo` in Redis using SHA256 as the key.
    ///
    /// # Arguments
    ///
    /// * `sha256` - The SHA256 hash of the file.
    /// * `file_info` - The `FileInfo` object to store.
    ///
    /// # Returns
    ///
    /// * `Result<(), RedisError>` - Empty result or an error.
    pub async fn set_file_info(&self, sha256: &str, file_info: &FileInfo) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("file_info:{}", sha256);
        let value = serde_json::to_string(file_info)
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        conn.set(key, value).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }

    /// Retrieves `FileInfo` from Redis using SHA256 as the key.
    ///
    /// # Arguments
    ///
    /// * `sha256` - The SHA256 hash of the file.
    ///
    /// # Returns
    ///
    /// * `Result<Option<FileInfo>, RedisError>` - The `FileInfo` if found, otherwise `None`, or an error.
    pub async fn get_file_info_by_sha(&self, sha256: &str) -> Result<Option<FileInfo>, RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("file_info:{}", sha256);
        let value: Option<String> = conn.get(key).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        if let Some(json_str) = value {
            let file_info: FileInfo = serde_json::from_str(&json_str)
                .map_err(|e| RedisError::CommandError(e.to_string()))?;
            Ok(Some(file_info))
        } else {
            Ok(None)
        }
    }

    /// Deletes `FileInfo` from Redis using SHA256 as the key.
    ///
    /// # Arguments
    ///
    /// * `sha256` - The SHA256 hash of the file.
    ///
    /// # Returns
    ///
    /// * `Result<(), RedisError>` - Empty result or an error.
    pub async fn delete_file_info(&self, sha256: &str) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("file_info:{}", sha256);
        conn.del(key).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }

    /// Sets a mapping from UUID to SHA256.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the file.
    /// * `sha256` - The SHA256 hash of the file.
    ///
    /// # Returns
    ///
    /// * `Result<(), RedisError>` - Empty result or an error.
    pub async fn set_sha_uuid_mapping(&self, uuid: &Uuid, sha256: &str) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("uuid_sha:{}", uuid);
        conn.set(key, sha256).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }

    /// Retrieves the SHA256 hash associated with a given UUID.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the file.
    ///
    /// # Returns
    ///
    /// * `Result<Option<String>, RedisError>` - The SHA256 hash if found, otherwise `None`, or an error.
    pub async fn get_sha_by_uuid(&self, uuid: &Uuid) -> Result<Option<String>, RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("uuid_sha:{}", uuid);
        let sha: Option<String> = conn.get(key).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(sha)
    }

    /// Deletes the UUID to SHA256 mapping from Redis.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the file.
    ///
    /// # Returns
    ///
    /// * `Result<(), RedisError>` - Empty result or an error.
    pub async fn delete_sha_uuid_mapping(&self, uuid: &Uuid) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("uuid_sha:{}", uuid);
        conn.del(key).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }
}
