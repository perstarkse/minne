use axum::{
    extract::State,
    http::{Response, StatusCode},
    response::{Html, IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::ApiError,
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::user::User,
};

use super::render_block;

page_data!(AccountData, "auth/account.html", {
    user: User
});

pub async fn show_account_page(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    info!("{:?}", auth.current_user);

    let output = render_template(
        AccountData::template_name(),
        AccountData {
            user: auth.current_user.unwrap(),
        },
        state.templates,
    )?;

    Ok(output.into_response())
}

pub async fn set_api_key(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    // Early return if the user is not authenticated
    let user = auth.current_user.ok_or(ApiError::AuthRequired)?;

    // Generate and set the API key
    let api_key = User::set_api_key(&user.id, &state.surreal_db_client).await?;

    // Update the user's API key
    let updated_user = User {
        api_key: Some(api_key),
        ..user
    };

    // Render the API key section block
    let output = render_block(
        AccountData::template_name(),
        "api_key_section",
        AccountData { user: updated_user },
        state.templates,
    )?;

    Ok(output.into_response())
}
