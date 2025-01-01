use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect},
};
use axum_htmx::HxRedirect;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    error::{AppError, HtmlError},
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
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let output = render_template(
        AccountData::template_name(),
        AccountData { user },
        state.templates.clone(),
    )
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    Ok(output.into_response())
}

pub async fn set_api_key(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    // Generate and set the API key
    let api_key = User::set_api_key(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

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
        state.templates.clone(),
    )
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    Ok(output.into_response())
}

pub async fn delete_account(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    delete_item::<User>(&state.surreal_db_client, &user.id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    auth.logout_user();

    auth.session.destroy();

    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}
