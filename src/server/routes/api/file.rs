use crate::{error::ApiError, server::AppState, storage::types::file_info::FileInfo};
use axum::{extract::State, response::IntoResponse, Json};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use serde_json::json;
use tempfile::NamedTempFile;
use tracing::info;

#[derive(Debug, TryFromMultipart)]
pub struct FileUploadRequest {
    #[form_data(limit = "1000000")] // Example limit: ~1000 KB
    pub file: FieldData<NamedTempFile>,
}

/// Handler to upload a new file.
///
/// Route: POST /file
pub async fn upload_handler(
    State(state): State<AppState>,
    TypedMultipart(input): TypedMultipart<FileUploadRequest>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received an upload request");

    // Process the file upload
    let file_info = FileInfo::new(input.file, &state.surreal_db_client).await?;

    // Prepare the response JSON
    let response = json!({
        "id": file_info.id,
        "sha256": file_info.sha256,
        "path": file_info.path,
        "mime_type": file_info.mime_type,
    });

    info!("File uploaded successfully: {:?}", file_info);

    // Return the response with HTTP 200
    Ok((axum::http::StatusCode::OK, Json(response)))
}
