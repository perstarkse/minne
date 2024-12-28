use axum::{
    extract::State,
    http::{Response, StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
};
use axum_htmx::HxRedirect;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    error::ApiError,
    page_data,
    server::{routes::html::render_template, AppState},
    storage::{db::delete_item, types::user::User},
};

use super::render_block;

page_data!(AccountData, "auth/account.html", {
    user: User
});

pub async fn show_account_page(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let output = render_template(
        AccountData::template_name(),
        AccountData { user },
        state.templates,
    )?;

    Ok(output.into_response())
}

pub async fn set_api_key(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    // Early return if the user is not authenticated
    let user = auth.current_user.as_ref().ok_or(ApiError::AuthRequired)?;

    // Generate and set the API key
    let api_key = User::set_api_key(&user.id, &state.surreal_db_client).await?;

    auth.cache_clear_user(user.id.to_string());

    // Update the user's API key
    let updated_user = User {
        api_key: Some(api_key),
        ..user.clone()
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

pub async fn delete_account(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    // Early return if the user is not authenticated
    let user = auth.current_user.as_ref().ok_or(ApiError::AuthRequired)?;

    delete_item::<User>(&state.surreal_db_client, &user.id).await?;

    auth.logout_user();

    auth.session.destroy();

    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}
