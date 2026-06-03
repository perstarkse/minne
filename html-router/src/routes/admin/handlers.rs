use async_openai::types::ListModelResponse;
use axum::{
    extract::{Query, State},
    Form,
};
use serde::{Deserialize, Serialize};

use common::{
    error::AppError,
    storage::types::{
        analytics::Analytics,
        system_prompts::{
            DEFAULT_IMAGE_PROCESSING_PROMPT, DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT,
            DEFAULT_QUERY_SYSTEM_PROMPT,
        },
        system_settings::{SystemSettings, SystemSettingsPatch},
    },
    utils::embedding::EmbeddingBackend,
};
use tracing::info;

use crate::{
    html_state::HtmlState,
    middlewares::response_middleware::{TemplateResponse, TemplateResult},
};

#[derive(Serialize)]
pub struct AdminPanelData {
    settings: SystemSettings,
    analytics: Option<Analytics>,
    users: Option<i64>,
    default_query_prompt: String,
    default_image_prompt: String,
    available_models: Option<ListModelResponse>,
    current_section: AdminSection,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdminSection {
    #[default]
    Overview,
    Models,
}


#[derive(Deserialize)]
pub struct AdminPanelQuery {
    section: Option<String>,
}

pub async fn show_admin_panel(
    State(state): State<HtmlState>,
    Query(query): Query<AdminPanelQuery>,
) -> TemplateResult {
    let section = match query.section.as_deref() {
        Some("models") => AdminSection::Models,
        _ => AdminSection::Overview,
    };

    let settings = SystemSettings::get_current(&state.db).await?;

    let (analytics, users) = if section == AdminSection::Overview {
        let (analytics, users) = tokio::try_join!(
            Analytics::get_current(&state.db),
            Analytics::get_users_amount(&state.db)
        )?;
        (Some(analytics), Some(users))
    } else {
        (None, None)
    };

    let available_models = if section == AdminSection::Models {
        Some(
            state
                .openai_client
                .models()
                .list()
                .await
                .map_err(|e| AppError::InternalError(e.to_string()))?,
        )
    } else {
        None
    };

    Ok(TemplateResponse::new_template(
        "admin/base.html",
        AdminPanelData {
            settings,
            analytics,
            available_models,
            users,
            default_query_prompt: DEFAULT_QUERY_SYSTEM_PROMPT.to_string(),
            default_image_prompt: DEFAULT_IMAGE_PROCESSING_PROMPT.to_string(),
            current_section: section,
        },
    ))
}

fn checkbox_to_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(|s| s == "on")
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
    Form(input): Form<RegistrationToggleInput>,
) -> TemplateResult {
    let new_settings = SystemSettingsPatch {
        registrations_enabled: Some(input.registration_open),
        ..Default::default()
    }
    .apply(&state.db)
    .await?;

    Ok(TemplateResponse::new_partial(
        "admin/sections/overview.html",
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
    image_processing_model: String,
    voice_processing_model: String,
    embedding_model: Option<String>,
    embedding_dimensions: Option<u32>,
}

#[derive(Serialize)]
pub struct ModelSettingsData {
    settings: SystemSettings,
    available_models: ListModelResponse,
}

pub async fn update_model_settings(
    State(state): State<HtmlState>,
    Form(input): Form<ModelSettingsInput>,
) -> TemplateResult {
    let current_settings = SystemSettings::get_current(&state.db).await?;

    // Check if using FastEmbed - if so, embedding model/dimensions cannot be changed via UI
    let uses_local_embeddings = current_settings.embedding_backend.is_some_and(
        |backend| matches!(backend, EmbeddingBackend::FastEmbed | EmbeddingBackend::Hashed),
    );

    // For local embeddings, ignore any embedding model/dimension changes from the form
    let (final_embedding_model, final_embedding_dimensions, reembedding_needed) =
        if uses_local_embeddings {
            // Keep current values - they're controlled by config, not the admin UI
            info!(
                backend = ?current_settings.embedding_backend,
                "Embedding model/dimensions controlled by config, ignoring form input"
            );
            (
                current_settings.embedding_model.clone(),
                current_settings.embedding_dimensions,
                false,
            )
        } else {
            // OpenAI backend - allow changes from form
            let reembedding_needed = input
                .embedding_dimensions
                .is_some_and(|new_dims| new_dims != current_settings.embedding_dimensions);
            (
                input
                    .embedding_model
                    .unwrap_or_else(|| current_settings.embedding_model.clone()),
                input
                    .embedding_dimensions
                    .unwrap_or(current_settings.embedding_dimensions),
                reembedding_needed,
            )
        };

    let new_settings = SystemSettingsPatch {
        query_model: Some(input.query_model),
        processing_model: Some(input.processing_model),
        image_processing_model: Some(input.image_processing_model),
        voice_processing_model: Some(input.voice_processing_model),
        embedding_model: Some(final_embedding_model),
        embedding_dimensions: Some(final_embedding_dimensions),
        ..Default::default()
    }
    .apply(&state.db)
    .await?;

    if reembedding_needed {
        // Re-embedding is owned by startup (the worker/combined binary), not the admin request.
        // Doing it inline here would leave the live, startup-built embedding provider embedding
        // queries at the old dimension while stored vectors move to the new one — broken retrieval
        // until restart. Persisting the new settings is enough: on the next restart the maintainer
        // detects the index/dimension mismatch and re-embeds before rebuilding indexes.
        info!(
            new_dimensions = new_settings.embedding_dimensions,
            "Embedding dimensions changed; restart the worker/server to re-embed and apply"
        );
    }

    let available_models = state
        .openai_client
        .models()
        .list()
        .await
        .map_err(|_e| AppError::InternalError("Failed to get models".to_string()))?;

    Ok(TemplateResponse::new_partial(
        "admin/sections/models.html",
        "model_settings_form",
        ModelSettingsData {
            settings: new_settings,
            available_models,
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
) -> TemplateResult {
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
    Form(input): Form<SystemPromptUpdateInput>,
) -> TemplateResult {
    let new_settings = SystemSettingsPatch {
        query_system_prompt: Some(input.query_system_prompt),
        ..Default::default()
    }
    .apply(&state.db)
    .await?;

    Ok(TemplateResponse::new_partial(
        "admin/sections/overview.html",
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
) -> TemplateResult {
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
    Form(input): Form<IngestionPromptUpdateInput>,
) -> TemplateResult {
    let new_settings = SystemSettingsPatch {
        ingestion_system_prompt: Some(input.ingestion_system_prompt),
        ..Default::default()
    }
    .apply(&state.db)
    .await?;

    Ok(TemplateResponse::new_partial(
        "admin/sections/overview.html",
        "system_prompt_section",
        SystemPromptSectionData {
            settings: new_settings,
        },
    ))
}

#[derive(Serialize)]
pub struct ImagePromptEditData {
    settings: SystemSettings,
    default_image_prompt: String,
}

pub async fn show_edit_image_prompt(
    State(state): State<HtmlState>,
) -> TemplateResult {
    let settings = SystemSettings::get_current(&state.db).await?;

    Ok(TemplateResponse::new_template(
        "admin/edit_image_prompt_modal.html",
        ImagePromptEditData {
            settings,
            default_image_prompt: DEFAULT_IMAGE_PROCESSING_PROMPT.to_string(),
        },
    ))
}

#[derive(Deserialize)]
pub struct ImagePromptUpdateInput {
    image_processing_prompt: String,
}

pub async fn patch_image_prompt(
    State(state): State<HtmlState>,
    Form(input): Form<ImagePromptUpdateInput>,
) -> TemplateResult {
    let new_settings = SystemSettingsPatch {
        image_processing_prompt: Some(input.image_processing_prompt),
        ..Default::default()
    }
    .apply(&state.db)
    .await?;

    Ok(TemplateResponse::new_partial(
        "admin/sections/overview.html",
        "system_prompt_section",
        SystemPromptSectionData {
            settings: new_settings,
        },
    ))
}
