use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use url::Url;
use uuid::Uuid;
use crate::redis::client::RedisClient;

use super::{file_info::FileInfo, ingress_object::IngressObject };


/// Struct defining the expected body when ingressing content.
#[derive(Serialize, Deserialize, Debug)]
pub struct IngressInput {
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,
    pub files: Option<Vec<String>>,
}

/// Error types for processing ingress content.
#[derive(Error, Debug)]
pub enum IngressContentError {
    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("MIME type detection failed for input: {0}")]
    MimeDetection(String),

    #[error("Unsupported MIME type: {0}")]
    UnsupportedMime(String),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("UUID parse error: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("Redis error: {0}")]
    RedisError(String),
}

/// Function to create ingress objects from input.
///
/// # Arguments
/// * `input` - IngressInput containing information needed to ingress content.
/// * `redis_client` - Initialized redis client needed to retrieve file information
///
/// # Returns
/// * `Vec<IngressObject>` - An array containing the ingressed objects, one file/contenttype per object.
pub async fn create_ingress_objects(
    input: IngressInput,
    redis_client: &RedisClient,
) -> Result<Vec<IngressObject>, IngressContentError> {
    // Initialize list
    let mut object_list = Vec::new();

    // Create a IngressObject from input.content if it exists, checking for URL or text
    if let Some(input_content) = input.content {
        match Url::parse(&input_content) {
            Ok(url) => {
                info!("Detected URL: {}", url);
                object_list.push(IngressObject::Url {
                    url: url.to_string(),
                    instructions: input.instructions.clone(),
                    category: input.category.clone(),
                });
            }
            Err(_) => {
                info!("Treating input as plain text");
                object_list.push(IngressObject::Text {
                    text: input_content.to_string(),
                    instructions: input.instructions.clone(),
                    category: input.category.clone(),
                });
            }
        }
    }

    // Look up FileInfo objects using the redis db and the submitted uuids in input.files
    if let Some(file_uuids) = input.files {
        for uuid_str in file_uuids {
            let uuid = Uuid::parse_str(&uuid_str)?;
            match FileInfo::get(uuid, redis_client).await {
                Ok(file_info) => {
                    object_list.push(IngressObject::File {
                        file_info,
                        instructions: input.instructions.clone(),
                        category: input.category.clone(),
                    });
                }
                Err(_) => {
                    info!("No file with UUID: {}", uuid);
                    // Optionally, you can collect errors or continue silently
                }
            }
        }
    }

    // If no objects are constructed, we return Err
    if object_list.is_empty() {
        return Err(IngressContentError::MimeDetection(
            "No valid content or files provided".into(),
        ));
    }

    Ok(object_list)
}

