use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
        user::User,
    },
    utils::ingest_limits::{validate_ingest_input, IngestValidationError},
};
use futures::{future::try_join_all, TryFutureExt};
use serde_json::json;
use tempfile::NamedTempFile;
use tracing::info;

use crate::{api_state::ApiState, error::ApiErr};

#[derive(Debug, TryFromMultipart)]
pub struct Params {
    pub content: Option<String>,
    pub context: String,
    pub category: String,
    #[form_data(limit = "20000000")]
    #[form_data(default)]
    pub files: Vec<FieldData<NamedTempFile>>,
}

pub async fn handle(
    State(state): State<ApiState>,
    Extension(user): Extension<User>,
    TypedMultipart(input): TypedMultipart<Params>,
) -> Result<impl IntoResponse, ApiErr> {
    let user_id = user.id;
    let has_content = input.content.as_ref().is_some_and(|c| !c.trim().is_empty());

    match validate_ingest_input(
        &state.config,
        input.content.as_deref(),
        &input.context,
        &input.category,
        input.files.len(),
    ) {
        Ok(()) => {}
        Err(IngestValidationError::PayloadTooLarge(message)) => {
            return Err(ApiErr::PayloadTooLarge(message));
        }
        Err(IngestValidationError::BadRequest(message)) => {
            return Err(ApiErr::ValidationError(message));
        }
    }

    info!(
        user_id = %user_id,
        has_content,
        content_len = input.content.as_ref().map_or(0, String::len),
        context_len = input.context.len(),
        category_len = input.category.len(),
        file_count = input.files.len(),
        "Received ingest request"
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
        user_id.clone(),
    )?;

    IngestionTask::create_all_and_add_to_db(payloads, &user_id, &state.db).await?;

    Ok((StatusCode::OK, Json(json!({ "status": "success" }))))
}
