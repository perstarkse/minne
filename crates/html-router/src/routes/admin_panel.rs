use axum::{
    extract::State,
    response::{IntoResponse, Redirect},
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use common::{
    error::HtmlError,
    storage::types::{analytics::Analytics, system_settings::SystemSettings, user::User},
};

use crate::{html_state::HtmlState, page_data};

use super::{render_block, render_template};

page_data!(AdminPanelData, "auth/admin_panel.html", {
    user: User,
    settings: SystemSettings,
    analytics: Analytics,
    users: i64,
});

pub async fn show_admin_panel(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated and admin
    let user = match auth.current_user {
        Some(user) if user.admin => user,
        _ => return Ok(Redirect::to("/").into_response()),
    };

    let settings = SystemSettings::get_current(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let analytics = Analytics::get_current(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let users_count = Analytics::get_users_amount(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        AdminPanelData::template_name(),
        AdminPanelData {
            user,
            settings,
            analytics,
            users: users_count,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
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
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(input): Form<RegistrationToggleInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated and admin
    let _user = match auth.current_user {
        Some(user) if user.admin => user,
        _ => return Ok(Redirect::to("/").into_response()),
    };

    let current_settings = SystemSettings::get_current(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let new_settings = SystemSettings {
        registrations_enabled: input.registration_open,
        ..current_settings.clone()
    };

    SystemSettings::update(&state.surreal_db_client, new_settings.clone())
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_block(
        AdminPanelData::template_name(),
        "registration_status_input",
        RegistrationToggleData {
            settings: new_settings,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
