use axum::{extract::State, response::IntoResponse, Form};
use chrono_tz::TZ_VARIANTS;
use serde::{Deserialize, Serialize};

use crate::{
    middleware_auth::RequireUser,
    template_response::{HtmlError, TemplateResponse},
    AuthSessionType,
};
use common::storage::types::user::User;

use crate::html_state::HtmlState;

#[derive(Serialize)]
pub struct AccountPageData {
    user: User,
    timezones: Vec<String>,
}

pub async fn show_account_page(
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let timezones = TZ_VARIANTS.iter().map(|tz| tz.to_string()).collect();

    Ok(TemplateResponse::new_template(
        "auth/account_settings.html",
        AccountPageData { user, timezones },
    ))
}

pub async fn set_api_key(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    auth: AuthSessionType,
) -> Result<impl IntoResponse, HtmlError> {
    // Generate and set the API key
    let api_key = User::set_api_key(&user.id, &state.db).await?;

    // Clear the cache so new requests have access to the user with api key
    auth.cache_clear_user(user.id.to_string());

    // Update the user's API key
    let updated_user = User {
        api_key: Some(api_key),
        ..user.clone()
    };

    // Render the API key section block
    Ok(TemplateResponse::new_partial(
        "auth/account_settings.html",
        "api_key_section",
        AccountPageData {
            user: updated_user,
            timezones: vec![],
        },
    ))
}

pub async fn delete_account(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    auth: AuthSessionType,
) -> Result<impl IntoResponse, HtmlError> {
    state.db.delete_item::<User>(&user.id).await?;

    auth.logout_user();

    auth.session.destroy();

    Ok(TemplateResponse::redirect("/"))
}

#[derive(Deserialize)]
pub struct UpdateTimezoneForm {
    timezone: String,
}

pub async fn update_timezone(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    auth: AuthSessionType,
    Form(form): Form<UpdateTimezoneForm>,
) -> Result<impl IntoResponse, HtmlError> {
    User::update_timezone(&user.id, &form.timezone, &state.db).await?;

    // Clear the cache
    auth.cache_clear_user(user.id.to_string());

    // Update the user's API key
    let updated_user = User {
        timezone: form.timezone,
        ..user.clone()
    };

    let timezones = TZ_VARIANTS.iter().map(|tz| tz.to_string()).collect();

    // Render the API key section block
    Ok(TemplateResponse::new_partial(
        "auth/account_settings.html",
        "timezone_section",
        AccountPageData {
            user: updated_user,
            timezones,
        },
    ))
}
