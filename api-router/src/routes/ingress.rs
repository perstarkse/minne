use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
        user::User,
    },
};
use futures::{future::try_join_all, TryFutureExt};
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
    info!("Received input: {:?}", input);

    let file_infos = try_join_all(input.files.into_iter().map(|file| {
        FileInfo::new(file, &state.db, &user.id, &state.config).map_err(AppError::from)
    }))
    .await?;

    let payloads = IngestionPayload::create_ingestion_payload(
        input.content,
        input.context,
        input.category,
        file_infos,
        user.id.as_str(),
    )?;

    let futures: Vec<_> = payloads
        .into_iter()
        .map(|object| {
            IngestionTask::create_and_add_to_db(object.clone(), user.id.clone(), &state.db)
        })
        .collect();

    try_join_all(futures).await.map_err(AppError::from)?;

    Ok(StatusCode::OK)
}
