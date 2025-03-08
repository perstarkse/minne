use axum::{extract::State, response::IntoResponse, Form};
use serde::{Deserialize, Serialize};

use crate::{
    middleware_auth::RequireUser,
    template_response::{HtmlError, TemplateResponse},
};
use common::storage::types::{analytics::Analytics, system_settings::SystemSettings, user::User};

use crate::html_state::HtmlState;

#[derive(Serialize)]
pub struct AdminPanelData {
    user: User,
    settings: SystemSettings,
    analytics: Analytics,
    users: i64,
}

pub async fn show_admin_panel(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let settings = SystemSettings::get_current(&state.db).await?;
    let analytics = Analytics::get_current(&state.db).await?;
    let users_count = Analytics::get_users_amount(&state.db).await?;

    Ok(TemplateResponse::new_template(
        "auth/admin_panel.html",
        AdminPanelData {
            user,
            settings,
            analytics,
            users: users_count,
        },
    ))
}

fn checkbox_to_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match String::deserialize(deserializer) {
        Ok(string) => Ok(string == "on"),
        Err(_) => Ok(false),
    }
}

#[derive(Deserialize)]
pub struct RegistrationToggleInput {
    #[serde(default)]
    #[serde(deserialize_with = "checkbox_to_bool")]
    registration_open: bool,
}

#[derive(Serialize)]
pub struct RegistrationToggleData {
    settings: SystemSettings,
}

pub async fn toggle_registration_status(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(input): Form<RegistrationToggleInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let current_settings = SystemSettings::get_current(&state.db).await?;

    let new_settings = SystemSettings {
        registrations_enabled: input.registration_open,
        ..current_settings.clone()
    };

    SystemSettings::update(&state.db, new_settings.clone()).await?;

    Ok(TemplateResponse::new_partial(
        "auth/admin_panel.html",
        "registration_status_input",
        RegistrationToggleData {
            settings: new_settings,
        },
    ))
}
