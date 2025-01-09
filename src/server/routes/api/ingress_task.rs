use crate::{error::ApiError, server::AppState, storage::types::user::User};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};

pub async fn get_queue_tasks(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<impl IntoResponse, ApiError> {
    let user_tasks = state
        .job_queue
        .get_user_jobs(&user.id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(user_tasks))
}

pub async fn delete_queue_task(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .job_queue
        .delete_job(&id, &user.id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::OK)
}
