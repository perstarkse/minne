use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
        user::User,
    },
};
use futures::{future::try_join_all, TryFutureExt};
use serde_json::json;
use tempfile::NamedTempFile;
use tracing::info;

use crate::{api_state::ApiState, error::ApiError};

#[derive(Debug, TryFromMultipart)]
pub struct IngestParams {
    pub content: Option<String>,
    pub context: String,
    pub category: String,
    #[form_data(limit = "10000000")] // Adjust limit as needed
    #[form_data(default)]
    pub files: Vec<FieldData<NamedTempFile>>,
}

pub async fn ingest_data(
    State(state): State<ApiState>,
    Extension(user): Extension<User>,
    TypedMultipart(input): TypedMultipart<IngestParams>,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = user.id;
    let content_bytes = input.content.as_ref().map_or(0, |c| c.len());
    let has_content = input.content.as_ref().is_some_and(|c| !c.trim().is_empty());
    let context_bytes = input.context.len();
    let category_bytes = input.category.len();
    let file_count = input.files.len();

    info!(
        user_id = %user_id,
        has_content,
        content_bytes,
        context_bytes,
        category_bytes,
        file_count,
        "Received ingestion request"
    );

    let file_infos = try_join_all(input.files.into_iter().map(|file| {
        FileInfo::new_with_storage(file, &state.db, &user_id, &state.storage)
            .map_err(AppError::from)
    }))
    .await?;

    let payloads = IngestionPayload::create_ingestion_payload(
        input.content,
        input.context,
        input.category,
        file_infos,
        &user_id,
    )?;

    let futures: Vec<_> = payloads
        .into_iter()
        .map(|object| IngestionTask::create_and_add_to_db(object, user_id.clone(), &state.db))
        .collect();

    try_join_all(futures).await?;

    Ok((StatusCode::OK, Json(json!({ "status": "success" }))))
}
