use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect},
    Form,
};
use axum_htmx::HxRedirect;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use chrono_tz::TZ_VARIANTS;
use surrealdb::{engine::any::Any, Surreal};

use common::{
    error::{AppError, HtmlError},
    storage::types::user::User,
};

use crate::{html_state::HtmlState, page_data};

use super::{render_block, render_template};

page_data!(AccountData, "auth/account_settings.html", {
    user: User,
    timezones: Vec<String>
});

pub async fn show_account_page(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let timezones = TZ_VARIANTS.iter().map(|tz| tz.to_string()).collect();

    let output = render_template(
        AccountData::template_name(),
        AccountData { user, timezones },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn set_api_key(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    // Generate and set the API key
    let api_key = User::set_api_key(&user.id, &state.db)
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
        AccountData {
            user: updated_user,
            timezones: vec![],
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn delete_account(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    state
        .db
        .delete_item::<User>(&user.id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    auth.logout_user();

    auth.session.destroy();

    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}

#[derive(Deserialize)]
pub struct UpdateTimezoneForm {
    timezone: String,
}

pub async fn update_timezone(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<UpdateTimezoneForm>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    User::update_timezone(&user.id, &form.timezone, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    auth.cache_clear_user(user.id.to_string());

    // Update the user's API key
    let updated_user = User {
        timezone: form.timezone,
        ..user.clone()
    };

    let timezones = TZ_VARIANTS.iter().map(|tz| tz.to_string()).collect();

    // Render the API key section block
    let output = render_block(
        AccountData::template_name(),
        "timezone_section",
        AccountData {
            user: updated_user,
            timezones,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
