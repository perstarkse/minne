use axum::{extract::State, response::IntoResponse, Extension, Json};
use common::storage::types::user::User;

use crate::{api_state::ApiState, error::ApiErr};

pub async fn list(
    State(state): State<ApiState>,
    Extension(user): Extension<User>,
) -> Result<impl IntoResponse, ApiErr> {
    let categories = User::get_user_categories(&user.id, &state.db).await?;

    Ok(Json(categories))
}
