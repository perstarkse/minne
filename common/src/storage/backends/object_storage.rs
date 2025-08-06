use super::{FileMetadata, StorageBackend, StorageError, StoredFile};
use async_trait::async_trait;
use bytes::Bytes;
use object_store::{
    aws::{AmazonS3, AmazonS3Builder},
    ObjectStore, ObjectMeta,
};
use std::sync::Arc;
use uuid::Uuid;

/// S3-compatible object storage backend
#[derive(Debug)]
pub struct S3Backend {
    store: Arc<AmazonS3>,
    bucket: String,
    prefix: String,
}

impl S3Backend {
    pub async fn new(
        bucket: String,
        region: Option<String>,
        endpoint: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        prefix: Option<String>,
    ) -> Result<Self, StorageError> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&bucket);

        // Set region
        if let Some(region) = region {
            builder = builder.with_region(&region);
        }

        // Set custom endpoint (for S3-compatible services like MinIO, DigitalOcean Spaces, etc.)
        if let Some(endpoint) = endpoint {
            builder = builder.with_endpoint(&endpoint);
        }

        // Set credentials if provided, otherwise rely on environment/IAM roles
        if let Some(access_key) = access_key_id {
            builder = builder.with_access_key_id(&access_key);
        }
        
        if let Some(secret_key) = secret_access_key {
            builder = builder.with_secret_access_key(&secret_key);
        }

        // For S3-compatible services, we might need to set path-style addressing
        if endpoint.is_some() {
            builder = builder.with_virtual_hosted_style_request(false);
        }

        let store = builder.build()
            .map_err(|e| StorageError::Config(format!("Failed to create S3 client: {}", e)))?;

        let prefix = prefix.unwrap_or_else(|| "minne".to_string());

        Ok(Self {
            store: Arc::new(store),
            bucket,
            prefix,
        })
    }

    /// Generate the object key for a file: {prefix}/{user_id}/{file_id}/{filename}
    fn get_object_key(&self, user_id: &str, file_id: &Uuid, filename: &str) -> String {
        format!("{}/{}/{}/{}", self.prefix, user_id, file_id, filename)
    }

    /// Parse an object key to extract components
    fn parse_object_key(&self, key: &str) -> Result<(String, String, String), StorageError> {
        let parts: Vec<&str> = key.splitn(4, '/').collect();
        if parts.len() < 4 {
            return Err(StorageError::Config(format!("Invalid object key format: {}", key)));
        }
        
        // Skip prefix check for now, just parse the last 3 parts
        let user_id = parts[1].to_string();
        let file_id = parts[2].to_string();
        let filename = parts[3].to_string();
        
        Ok((user_id, file_id, filename))
    }
}

#[async_trait]
impl StorageBackend for S3Backend {
    async fn store_file(
        &self,
        user_id: &str,
        file_id: &Uuid,
        filename: &str,
        content: Bytes,
        content_type: Option<&str>,
    ) -> Result<String, StorageError> {
        let object_key = self.get_object_key(user_id, file_id, filename);
        let path = object_store::path::Path::from(object_key.clone());

        let mut put_opts = object_store::PutOptions::default();
        
        // Set content type if provided
        if let Some(ct) = content_type {
            put_opts.content_type = Some(ct.to_string());
        }

        self.store
            .put_opts(&path, content.into(), &put_opts)
            .await
            .map_err(StorageError::ObjectStore)?;

        Ok(object_key)
    }

    async fn get_file(&self, path: &str) -> Result<StoredFile, StorageError> {
        let object_path = object_store::path::Path::from(path);

        let get_result = self.store
            .get(&object_path)
            .await
            .map_err(StorageError::ObjectStore)?;

        let content = get_result.bytes().await.map_err(StorageError::ObjectStore)?;
        let metadata = self.get_metadata(path).await?;

        Ok(StoredFile { content, metadata })
    }

    async fn delete_file(&self, path: &str) -> Result<(), StorageError> {
        let object_path = object_store::path::Path::from(path);

        self.store
            .delete(&object_path)
            .await
            .map_err(StorageError::ObjectStore)?;

        Ok(())
    }

    async fn file_exists(&self, path: &str) -> Result<bool, StorageError> {
        let object_path = object_store::path::Path::from(path);

        match self.store.head(&object_path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(StorageError::ObjectStore(e)),
        }
    }

    async fn get_metadata(&self, path: &str) -> Result<FileMetadata, StorageError> {
        let object_path = object_store::path::Path::from(path);

        let head_result = self.store
            .head(&object_path)
            .await
            .map_err(StorageError::ObjectStore)?;

        let content_type = head_result.metadata.get("content-type")
            .map(|ct| ct.to_string());

        Ok(FileMetadata {
            size: head_result.size,
            content_type,
            last_modified: Some(head_result.last_modified),
        })
    }

    async fn list_files(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let search_prefix = if prefix.is_empty() {
            format!("{}/", self.prefix)
        } else {
            format!("{}/{}", self.prefix, prefix)
        };

        let list_stream = self.store
            .list(Some(&object_store::path::Path::from(search_prefix)))
            .await
            .map_err(StorageError::ObjectStore)?;

        let mut files = Vec::new();
        use futures::TryStreamExt;
        
        let objects: Vec<ObjectMeta> = list_stream
            .try_collect()
            .await
            .map_err(StorageError::ObjectStore)?;

        for object in objects {
            files.push(object.location.to_string());
        }

        Ok(files)
    }
}

impl Clone for S3Backend {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            bucket: self.bucket.clone(),
            prefix: self.prefix.clone(),
        }
    }
}