use axum::{
    extract::Path,
    response::IntoResponse,
    Json,
    
};
use axum_typed_multipart::{TypedMultipart, FieldData, TryFromMultipart};
use serde_json::json;
use tempfile::NamedTempFile;
use tracing::info;
use uuid::Uuid;

use crate::{
    models::file_info::{FileError, FileInfo},
    redis::client::RedisClient, surrealdb::{document::set_file_info, SurrealDbClient},
};

#[derive(Debug, TryFromMultipart)]
pub struct FileUploadRequest {
    #[form_data(limit = "100000")] // Example limit: ~100 KB
    pub file: FieldData<NamedTempFile>,
}

/// Handler to upload a new file.
/// 
/// Route: POST /file
pub async fn upload_handler(
    TypedMultipart(input): TypedMultipart<FileUploadRequest>,
) -> Result<impl IntoResponse, FileError> {
    info!("Received an upload request");

    // Initialize a new RedisClient instance
    let redis_client = RedisClient::new("redis://127.0.0.1/");

    let database = SurrealDbClient::new().await.map_err(|e| FileError::PersistError(e.to_string())).unwrap();
    // Process the file upload
    let file_info = FileInfo::new(input.file, &database).await?;

    // Prepare the response JSON
    let response = json!({
        "uuid": file_info.uuid,
        "sha256": file_info.sha256,
        "path": file_info.path,
        "mime_type": file_info.mime_type,
    });

    info!("File uploaded successfully: {:?}", file_info);


    // Return the response with HTTP 200
    Ok((axum::http::StatusCode::OK, Json(response)))
}

/// Handler to retrieve file information by UUID.
/// 
/// Route: GET /file/:uuid
pub async fn get_file_handler(
    Path(uuid_str): Path<String>,
) -> Result<impl IntoResponse, FileError> {
    // Parse UUID
    let uuid = Uuid::parse_str(&uuid_str).map_err(|_| FileError::InvalidUuid(uuid_str.clone()))?;

    // Initialize the database client
    let db_client = SurrealDbClient::new().await.map_err(|e| FileError::PersistError(e.to_string())).unwrap();

    // Retrieve FileInfo
    let file_info = FileInfo::get_by_uuid(uuid, &db_client).await?;

    // Prepare the response JSON
    let response = json!({
        "uuid": file_info.uuid,
        "sha256": file_info.sha256,
        "path": file_info.path,
        "mime_type": file_info.mime_type,
    });

    info!("Retrieved FileInfo: {:?}", file_info);

    // Return the response with HTTP 200
    Ok((axum::http::StatusCode::OK, Json(response)))
}

/// Handler to update an existing file by UUID.
/// 
/// Route: PUT /file/:uuid
pub async fn update_file_handler(
    Path(uuid_str): Path<String>,
    TypedMultipart(input): TypedMultipart<FileUploadRequest>,
) -> Result<impl IntoResponse, FileError> {
    // Parse UUID
    let uuid = Uuid::parse_str(&uuid_str).map_err(|_| FileError::InvalidUuid(uuid_str.clone()))?;

    // Initialize RedisClient
    let redis_client = RedisClient::new("redis://127.0.0.1/");

    // Update the file
    let updated_file_info = FileInfo::update(uuid, input.file, &redis_client).await?;

    // Prepare the response JSON
    let response = json!({
        "uuid": updated_file_info.uuid,
        "sha256": updated_file_info.sha256,
        "path": updated_file_info.path,
        "mime_type": updated_file_info.mime_type,
    });

    info!("File updated successfully: {:?}", updated_file_info);

    // Return the response with HTTP 200
    Ok((axum::http::StatusCode::OK, Json(response)))
}

/// Handler to delete a file by UUID.
/// 
/// Route: DELETE /file/:uuid
pub async fn delete_file_handler(
    Path(uuid_str): Path<String>,
) -> Result<impl IntoResponse, FileError> {
    // Parse UUID
    let uuid = Uuid::parse_str(&uuid_str).map_err(|_| FileError::InvalidUuid(uuid_str.clone()))?;

    // Initialize RedisClient
    let redis_client = RedisClient::new("redis://127.0.0.1/");

    // Delete the file
    FileInfo::delete(uuid, &redis_client).await?;

    info!("Deleted file with UUID: {}", uuid);

    // Prepare the response JSON
    let response = json!({
        "message": "File deleted successfully",
    });

    // Return the response with HTTP 204 No Content
    Ok((axum::http::StatusCode::NO_CONTENT, Json(response)))
}
