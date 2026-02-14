use axum::{extract::State, response::IntoResponse, Form};
use chrono_tz::TZ_VARIANTS;
use serde::{Deserialize, Serialize};

use crate::{
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
    AuthSessionType,
};
use common::storage::types::user::{Theme, User};

use crate::html_state::HtmlState;

#[derive(Serialize)]
pub struct AccountPageData {
    timezones: Vec<String>,
    theme_options: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_theme: Option<String>,
}

pub async fn show_account_page(
    RequireUser(user): RequireUser,
    State(_state): State<HtmlState>,
) -> Result<impl IntoResponse, HtmlError> {
    let timezones = TZ_VARIANTS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    let theme_options = vec![
        Theme::Light.as_str().to_string(),
        Theme::Dark.as_str().to_string(),
        Theme::WarmPaper.as_str().to_string(),
        Theme::ObsidianPrism.as_str().to_string(),
        Theme::System.as_str().to_string(),
    ];

    Ok(TemplateResponse::new_template(
        "auth/account_settings.html",
        AccountPageData {
            timezones,
            theme_options,
            api_key: user.api_key,
            selected_timezone: None,
            selected_theme: None,
        },
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

    // Render the API key section block
    Ok(TemplateResponse::new_partial(
        "auth/account_settings.html",
        "api_key_section",
        AccountPageData {
            timezones: vec![],
            theme_options: vec![],
            api_key: Some(api_key),
            selected_timezone: None,
            selected_theme: None,
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

    let timezones = TZ_VARIANTS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    // Render the API key section block
    Ok(TemplateResponse::new_partial(
        "auth/account_settings.html",
        "timezone_section",
        AccountPageData {
            timezones,
            theme_options: vec![],
            api_key: None,
            selected_timezone: Some(form.timezone),
            selected_theme: None,
        },
    ))
}

#[derive(Deserialize)]
pub struct UpdateThemeForm {
    theme: String,
}

pub async fn update_theme(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    auth: AuthSessionType,
    Form(form): Form<UpdateThemeForm>,
) -> Result<impl IntoResponse, HtmlError> {
    User::update_theme(&user.id, &form.theme, &state.db).await?;

    // Clear the cache
    auth.cache_clear_user(user.id.to_string());

    let theme_options = vec![
        Theme::Light.as_str().to_string(),
        Theme::Dark.as_str().to_string(),
        Theme::WarmPaper.as_str().to_string(),
        Theme::ObsidianPrism.as_str().to_string(),
        Theme::System.as_str().to_string(),
    ];

    Ok(TemplateResponse::new_partial(
        "auth/account_settings.html",
        "theme_section",
        AccountPageData {
            timezones: vec![],
            theme_options,
            api_key: None,
            selected_timezone: None,
            selected_theme: Some(form.theme),
        },
    ))
}

pub async fn show_change_password(
    RequireUser(_user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    Ok(TemplateResponse::new_template(
        "auth/change_password_form.html",
        (),
    ))
}

#[derive(Deserialize)]
pub struct NewPasswordForm {
    old_password: String,
    new_password: String,
}

pub async fn change_password(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    auth: AuthSessionType,
    Form(form): Form<NewPasswordForm>,
) -> Result<impl IntoResponse, HtmlError> {
    // Authenticate to make sure the password matches
    let authenticated_user = User::authenticate(&user.email, &form.old_password, &state.db).await?;

    User::patch_password(&authenticated_user.email, &form.new_password, &state.db).await?;

    auth.cache_clear_user(user.id);

    Ok(TemplateResponse::new_partial(
        "auth/account_settings.html",
        "change_password_section",
        (),
    ))
}
