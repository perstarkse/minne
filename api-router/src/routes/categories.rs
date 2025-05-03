use axum::{extract::State, response::IntoResponse, Extension, Json};
use common::storage::types::user::User;

use crate::{api_state::ApiState, error::ApiError};

pub async fn get_categories(
    State(state): State<ApiState>,
    Extension(user): Extension<User>,
) -> Result<impl IntoResponse, ApiError> {
    let categories = User::get_user_categories(&user.id, &state.db).await?;

    Ok(Json(categories))
}
