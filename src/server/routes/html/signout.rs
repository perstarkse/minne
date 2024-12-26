use axum::response::{IntoResponse, Redirect};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use crate::{error::ApiError, storage::types::user::User};

pub async fn sign_out_user(
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    auth.logout_user();

    Ok(Redirect::to("/").into_response())
}
