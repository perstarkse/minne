use axum::async_trait;
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

/// Defines the behavior for Redis client operations.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RedisClientTrait: Send + Sync {
    /// Establishes a new multiplexed asynchronous connection to Redis.
    async fn get_connection(&self) -> Result<redis::aio::MultiplexedConnection, RedisError>;

    /// Stores `FileInfo` in Redis using SHA256 as the key.
    async fn set_file_info(&self, sha256: &str, file_info: &FileInfo) -> Result<(), RedisError>;

    /// Retrieves `FileInfo` from Redis using SHA256 as the key.
    async fn get_file_info_by_sha(&self, sha256: &str) -> Result<Option<FileInfo>, RedisError>;

    /// Deletes `FileInfo` from Redis using SHA256 as the key.
    async fn delete_file_info(&self, sha256: &str) -> Result<(), RedisError>;

    /// Sets a mapping from UUID to SHA256.
    async fn set_sha_uuid_mapping(&self, uuid: &Uuid, sha256: &str) -> Result<(), RedisError>;

    /// Retrieves the SHA256 hash associated with a given UUID.
    async fn get_sha_by_uuid(&self, uuid: &Uuid) -> Result<Option<String>, RedisError>;

    /// Deletes the UUID to SHA256 mapping from Redis.
    async fn delete_sha_uuid_mapping(&self, uuid: &Uuid) -> Result<(), RedisError>;
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
}

#[async_trait]
impl RedisClientTrait for RedisClient {
    /// Establishes a new multiplexed asynchronous connection to Redis.
    ///
    /// # Returns
    /// * `MultiplexedConnection` - The established connection.
    async fn get_connection(&self) -> Result<redis::aio::MultiplexedConnection, RedisError> {
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
    /// * `sha256` - The SHA256 hash of the file.
    /// * `file_info` - The `FileInfo` object to store.
    ///
    /// # Returns
    /// * `Result<(), RedisError>` - Empty result or an error.
    async fn set_file_info(&self, sha256: &str, file_info: &FileInfo) -> Result<(), RedisError> {
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
    /// * `sha256` - The SHA256 hash of the file.
    ///
    /// # Returns
    /// * `Result<Option<FileInfo>, RedisError>` - The `FileInfo` if found, otherwise `None`, or an error.
    async fn get_file_info_by_sha(&self, sha256: &str) -> Result<Option<FileInfo>, RedisError> {
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
    /// * `sha256` - The SHA256 hash of the file.
    ///
    /// # Returns
    /// * `Result<(), RedisError>` - Empty result or an error.
    async fn delete_file_info(&self, sha256: &str) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("file_info:{}", sha256);
        conn.del(key).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }

    /// Sets a mapping from UUID to SHA256.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file.
    /// * `sha256` - The SHA256 hash of the file.
    ///
    /// # Returns
    /// * `Result<(), RedisError>` - Empty result or an error.
    async fn set_sha_uuid_mapping(&self, uuid: &Uuid, sha256: &str) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("uuid_sha:{}", uuid);
        conn.set(key, sha256).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }

    /// Retrieves the SHA256 hash associated with a given UUID.
    ///
    /// # Arguments
    /// * `uuid` - The UUID of the file.
    ///
    /// # Returns
    /// * `Result<Option<String>, RedisError>` - The SHA256 hash if found, otherwise `None`, or an error.
    async fn get_sha_by_uuid(&self, uuid: &Uuid) -> Result<Option<String>, RedisError> {
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
    async fn delete_sha_uuid_mapping(&self, uuid: &Uuid) -> Result<(), RedisError> {
        let mut conn = self.get_connection().await?;
        let key = format!("uuid_sha:{}", uuid);
        conn.del(key).await
            .map_err(|e| RedisError::CommandError(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_set_file_info() {
        // Initialize the mock.
        let mut mock_redis = MockRedisClientTrait::new();

        let test_sha = "dummysha256hash".to_string();
        let test_file_info = FileInfo {
            uuid: Uuid::new_v4(),
            sha256: test_sha.clone(),
            path: "/path/to/file".to_string(),
            mime_type: "text/plain".to_string(),
        };

        // Setup expectation for `set_file_info`.
        mock_redis
            .expect_set_file_info()
            .with(eq(test_sha.clone()), eq(test_file_info.clone()))
            .times(1)
            .returning(|_, _| Ok(()) );

        // Call `set_file_info` on the mock.
        let set_result = mock_redis.set_file_info(&test_sha, &test_file_info).await;
        assert!(set_result.is_ok());
    }

    #[tokio::test]
    async fn test_get_file_info_by_sha() {
        // Initialize the mock.
        let mut mock_redis = MockRedisClientTrait::new();

        let test_sha = "dummysha256hash".to_string();
        let test_file_info = FileInfo {
            uuid: Uuid::new_v4(),
            sha256: test_sha.clone(),
            path: "/path/to/file".to_string(),
            mime_type: "text/plain".to_string(),
        };

        // Clone the FileInfo to use inside the closure.
        let fi_clone = test_file_info.clone();

        // Setup expectation for `get_file_info_by_sha`.
        mock_redis
            .expect_get_file_info_by_sha()
            .with(eq(test_sha.clone()))
            .times(1)
            .returning(move |_: &str| {
                // Return the cloned FileInfo.
                let fi_inner = fi_clone.clone();
                Ok(Some(fi_inner)) 
            });

        // Call `get_file_info_by_sha` on the mock.
        let get_result = mock_redis.get_file_info_by_sha(&test_sha).await;
        assert!(get_result.is_ok());
        assert_eq!(get_result.unwrap(), Some(test_file_info));
    }

    #[tokio::test]
    async fn test_delete_file_info() {
        // Initialize the mock.
        let mut mock_redis = MockRedisClientTrait::new();

        let test_sha = "dummysha256hash".to_string();

        // Setup expectation for `delete_file_info`.
        mock_redis
            .expect_delete_file_info()
            .with(eq(test_sha.clone()))
            .times(1)
            .returning(|_: &str|  Ok(()) );

        // Call `delete_file_info` on the mock.
        let delete_result = mock_redis.delete_file_info(&test_sha).await;
        assert!(delete_result.is_ok());
    }

    #[tokio::test]
    async fn test_set_sha_uuid_mapping() {
        // Initialize the mock.
        let mut mock_redis = MockRedisClientTrait::new();

        let test_uuid = Uuid::new_v4();
        let test_sha = "dummysha256hash".to_string();

        // Setup expectation for `set_sha_uuid_mapping`.
        mock_redis
            .expect_set_sha_uuid_mapping()
            .with(eq(test_uuid.clone()), eq(test_sha.clone()))
            .times(1)
            .returning(|_, _|  Ok(()) );

        // Call `set_sha_uuid_mapping` on the mock.
        let set_result = mock_redis.set_sha_uuid_mapping(&test_uuid, &test_sha).await;
        assert!(set_result.is_ok());
    }

   #[tokio::test]
    async fn test_get_sha_by_uuid() {
        // Initialize the mock.
        let mut mock_redis = MockRedisClientTrait::new();

        let test_uuid = Uuid::new_v4();
        let test_sha = "dummysha256hash".to_string();

        // Clone the SHA to use inside the closure.
        let sha_clone = test_sha.clone();

        // Setup expectation for `get_sha_by_uuid`.
        mock_redis
            .expect_get_sha_by_uuid()
            .with(eq(test_uuid.clone()))
            .times(1)
            .returning(move |_: &Uuid| {
                let sha_inner = sha_clone.clone();
                 Ok(Some(sha_inner)) 
                             });

        // Call `get_sha_by_uuid` on the mock.
        let get_result = mock_redis.get_sha_by_uuid(&test_uuid).await;
        assert!(get_result.is_ok());
        assert_eq!(get_result.unwrap(), Some(test_sha));
    }


    #[tokio::test]
    async fn test_delete_sha_uuid_mapping() {
        // Initialize the mock.
        let mut mock_redis = MockRedisClientTrait::new();

        let test_uuid = Uuid::new_v4();

        // Setup expectation for `delete_sha_uuid_mapping`.
        mock_redis
            .expect_delete_sha_uuid_mapping()
            .with(eq(test_uuid.clone()))
            .times(1)
            .returning(|_: &Uuid| Ok(()) );

        // Call `delete_sha_uuid_mapping` on the mock.
        let delete_result = mock_redis.delete_sha_uuid_mapping(&test_uuid).await;
        assert!(delete_result.is_ok());
    }
}

