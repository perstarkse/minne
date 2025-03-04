use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use common::{
    error::{ApiError, AppError},
    ingress::ingress_input::{create_ingress_objects, IngressInput},
    storage::types::{file_info::FileInfo, user::User},
};
use futures::{future::try_join_all, TryFutureExt};
use tempfile::NamedTempFile;
use tracing::{debug, info};

use crate::api_state::ApiState;

#[derive(Debug, TryFromMultipart)]
pub struct IngressParams {
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,
    #[form_data(limit = "10000000")] // Adjust limit as needed
    #[form_data(default)]
    pub files: Vec<FieldData<NamedTempFile>>,
}

pub async fn ingress_data(
    State(state): State<ApiState>,
    Extension(user): Extension<User>,
    TypedMultipart(input): TypedMultipart<IngressParams>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", input);

    let file_infos = try_join_all(input.files.into_iter().map(|file| {
        FileInfo::new(file, &state.surreal_db_client, &user.id).map_err(AppError::from)
    }))
    .await?;

    debug!("Got file infos");

    let ingress_objects = create_ingress_objects(
        IngressInput {
            content: input.content,
            instructions: input.instructions,
            category: input.category,
            files: file_infos,
        },
        user.id.as_str(),
    )?;
    debug!("Got ingress objects");

    let futures: Vec<_> = ingress_objects
        .into_iter()
        .map(|object| state.job_queue.enqueue(object.clone(), user.id.clone()))
        .collect();

    try_join_all(futures).await.map_err(AppError::from)?;

    Ok(StatusCode::OK)
}
