use axum::{extract::State, response::IntoResponse, Form};
use serde::{Deserialize, Serialize};

use common::storage::types::{
    analytics::Analytics,
    system_prompts::{DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT, DEFAULT_QUERY_SYSTEM_PROMPT},
    system_settings::SystemSettings,
    user::User,
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

#[derive(Serialize)]
pub struct AdminPanelData {
    user: User,
    settings: SystemSettings,
    analytics: Analytics,
    users: i64,
    default_query_prompt: String,
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
            default_query_prompt: DEFAULT_QUERY_SYSTEM_PROMPT.to_string(),
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

#[derive(Deserialize)]
pub struct ModelSettingsInput {
    query_model: String,
    processing_model: String,
}

#[derive(Serialize)]
pub struct ModelSettingsData {
    settings: SystemSettings,
}

pub async fn update_model_settings(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(input): Form<ModelSettingsInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let current_settings = SystemSettings::get_current(&state.db).await?;

    let new_settings = SystemSettings {
        query_model: input.query_model,
        processing_model: input.processing_model,
        ..current_settings
    };

    SystemSettings::update(&state.db, new_settings.clone()).await?;

    Ok(TemplateResponse::new_partial(
        "auth/admin_panel.html",
        "model_settings_form",
        ModelSettingsData {
            settings: new_settings,
        },
    ))
}

#[derive(Serialize)]
pub struct SystemPromptEditData {
    settings: SystemSettings,
    default_query_prompt: String,
}

pub async fn show_edit_system_prompt(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let settings = SystemSettings::get_current(&state.db).await?;

    Ok(TemplateResponse::new_template(
        "admin/edit_query_prompt_modal.html",
        SystemPromptEditData {
            settings,
            default_query_prompt: DEFAULT_QUERY_SYSTEM_PROMPT.to_string(),
        },
    ))
}

#[derive(Deserialize)]
pub struct SystemPromptUpdateInput {
    query_system_prompt: String,
}

#[derive(Serialize)]
pub struct SystemPromptSectionData {
    settings: SystemSettings,
}

pub async fn patch_query_prompt(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(input): Form<SystemPromptUpdateInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let current_settings = SystemSettings::get_current(&state.db).await?;

    let new_settings = SystemSettings {
        query_system_prompt: input.query_system_prompt,
        ..current_settings.clone()
    };

    SystemSettings::update(&state.db, new_settings.clone()).await?;

    Ok(TemplateResponse::new_partial(
        "auth/admin_panel.html",
        "system_prompt_section",
        SystemPromptSectionData {
            settings: new_settings,
        },
    ))
}

#[derive(Serialize)]
pub struct IngestionPromptEditData {
    settings: SystemSettings,
    default_ingestion_prompt: String,
}

pub async fn show_edit_ingestion_prompt(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let settings = SystemSettings::get_current(&state.db).await?;

    Ok(TemplateResponse::new_template(
        "admin/edit_ingestion_prompt_modal.html",
        IngestionPromptEditData {
            settings,
            default_ingestion_prompt: DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT.to_string(),
        },
    ))
}

#[derive(Deserialize)]
pub struct IngestionPromptUpdateInput {
    ingestion_system_prompt: String,
}

pub async fn patch_ingestion_prompt(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(input): Form<IngestionPromptUpdateInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let current_settings = SystemSettings::get_current(&state.db).await?;

    let new_settings = SystemSettings {
        ingestion_system_prompt: input.ingestion_system_prompt,
        ..current_settings.clone()
    };

    SystemSettings::update(&state.db, new_settings.clone()).await?;

    Ok(TemplateResponse::new_partial(
        "auth/admin_panel.html",
        "system_prompt_section",
        SystemPromptSectionData {
            settings: new_settings,
        },
    ))
}
