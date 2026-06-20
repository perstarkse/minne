use async_openai::types::models::ListModelResponse;
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
    utils::{
        config::AppConfig,
        embedding::{
            fastembed_model_dimension, is_valid_fastembed_model_code,
            list_fastembed_embedding_models, EmbeddingBackend, FastEmbedModelOption,
        },
    },
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
    fastembed_models: Option<Vec<FastEmbedModelOption>>,
    fastembed_model_locked_by_config: bool,
    effective_embedding_backend: String,
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

    let (available_models, fastembed_models, fastembed_model_locked_by_config) =
        if section == AdminSection::Models {
            let available_models = Some(
                state
                    .openai_client
                    .models()
                    .list()
                    .await
                    .map_err(|e| AppError::InternalError(e.to_string()))?,
            );
            let fastembed_models = is_fastembed_admin_context(&settings, &state.config)
                .then(list_fastembed_embedding_models);
            let fastembed_model_locked_by_config = state.config.fastembed_model.is_some();
            (
                available_models,
                fastembed_models,
                fastembed_model_locked_by_config,
            )
        } else {
            (None, None, false)
        };

    let effective_backend = effective_embedding_backend(&settings, &state.config)
        .as_str()
        .to_string();

    Ok(TemplateResponse::new_template(
        "admin/base.html",
        AdminPanelData {
            settings,
            analytics,
            available_models,
            fastembed_models,
            fastembed_model_locked_by_config,
            effective_embedding_backend: effective_backend,
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
    fastembed_models: Option<Vec<FastEmbedModelOption>>,
    fastembed_model_locked_by_config: bool,
    effective_embedding_backend: String,
}

struct EmbeddingSettingsPlan {
    embedding_model: String,
    embedding_dimensions: u32,
    reembedding_needed: bool,
    restart_needed: bool,
}

fn effective_embedding_backend(settings: &SystemSettings, config: &AppConfig) -> EmbeddingBackend {
    settings
        .embedding_backend
        .unwrap_or(config.embedding_backend)
}

fn is_fastembed_admin_context(settings: &SystemSettings, config: &AppConfig) -> bool {
    effective_embedding_backend(settings, config) == EmbeddingBackend::FastEmbed
}

fn plan_embedding_settings_update(
    current: &SystemSettings,
    input: &ModelSettingsInput,
    config: &AppConfig,
) -> Result<EmbeddingSettingsPlan, AppError> {
    match effective_embedding_backend(current, config) {
        EmbeddingBackend::OpenAI => {
            let reembedding_needed = input
                .embedding_dimensions
                .is_some_and(|new_dims| new_dims != current.embedding_dimensions);
            let embedding_model = input
                .embedding_model
                .clone()
                .unwrap_or_else(|| current.embedding_model.clone());
            let embedding_dimensions = input
                .embedding_dimensions
                .unwrap_or(current.embedding_dimensions);
            Ok(EmbeddingSettingsPlan {
                embedding_model,
                embedding_dimensions,
                reembedding_needed,
                restart_needed: reembedding_needed,
            })
        }
        EmbeddingBackend::FastEmbed => {
            if config.fastembed_model.is_some() {
                return Ok(EmbeddingSettingsPlan {
                    embedding_model: current.embedding_model.clone(),
                    embedding_dimensions: current.embedding_dimensions,
                    reembedding_needed: false,
                    restart_needed: false,
                });
            }

            let embedding_model = input
                .embedding_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map_or_else(|| current.embedding_model.clone(), ToOwned::to_owned);

            if !is_valid_fastembed_model_code(&embedding_model) {
                return Err(AppError::Validation(format!(
                    "Unknown FastEmbed model '{embedding_model}'. Choose a model from the list."
                )));
            }

            let embedding_dimensions =
                fastembed_model_dimension(&embedding_model).map_err(AppError::from)?;
            let reembedding_needed = embedding_dimensions != current.embedding_dimensions;
            let restart_needed = embedding_model != current.embedding_model || reembedding_needed;

            Ok(EmbeddingSettingsPlan {
                embedding_model,
                embedding_dimensions,
                reembedding_needed,
                restart_needed,
            })
        }
        EmbeddingBackend::Hashed => {
            info!(
                backend = ?current.embedding_backend,
                "Embedding model/dimensions for hashed backend are controlled by config"
            );
            Ok(EmbeddingSettingsPlan {
                embedding_model: current.embedding_model.clone(),
                embedding_dimensions: current.embedding_dimensions,
                reembedding_needed: false,
                restart_needed: false,
            })
        }
    }
}

pub async fn update_model_settings(
    State(state): State<HtmlState>,
    Form(input): Form<ModelSettingsInput>,
) -> TemplateResult {
    let current_settings = SystemSettings::get_current(&state.db).await?;
    let embedding_plan = plan_embedding_settings_update(&current_settings, &input, &state.config)?;

    let new_settings = SystemSettingsPatch {
        query_model: Some(input.query_model),
        processing_model: Some(input.processing_model),
        image_processing_model: Some(input.image_processing_model),
        voice_processing_model: Some(input.voice_processing_model),
        embedding_model: Some(embedding_plan.embedding_model),
        embedding_dimensions: Some(embedding_plan.embedding_dimensions),
        ..Default::default()
    }
    .apply(&state.db)
    .await?;

    if embedding_plan.reembedding_needed {
        // Re-embedding is owned by startup (the worker/combined binary), not the admin request.
        info!(
            new_dimensions = new_settings.embedding_dimensions,
            "Embedding dimensions changed; restart the worker/server to re-embed and apply"
        );
    } else if embedding_plan.restart_needed {
        info!(
            new_model = %new_settings.embedding_model,
            "Embedding model changed; restart the worker/server to apply"
        );
    }

    let available_models = state
        .openai_client
        .models()
        .list()
        .await
        .map_err(|_e| AppError::InternalError("Failed to get models".to_string()))?;

    let effective_backend = effective_embedding_backend(&new_settings, &state.config)
        .as_str()
        .to_string();
    let show_fastembed_models = is_fastembed_admin_context(&new_settings, &state.config)
        .then(list_fastembed_embedding_models);

    Ok(TemplateResponse::new_partial(
        "admin/sections/models.html",
        "model_settings_form",
        ModelSettingsData {
            settings: new_settings,
            available_models,
            fastembed_models: show_fastembed_models,
            fastembed_model_locked_by_config: state.config.fastembed_model.is_some(),
            effective_embedding_backend: effective_backend,
        },
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use common::utils::config::AppConfig;

    fn openai_settings() -> SystemSettings {
        SystemSettings {
            id: "current".into(),
            registrations_enabled: true,
            require_email_verification: false,
            query_model: "gpt-4o-mini".into(),
            processing_model: "gpt-4o-mini".into(),
            embedding_model: "text-embedding-3-small".into(),
            embedding_dimensions: 1536,
            embedding_backend: Some(EmbeddingBackend::OpenAI),
            query_system_prompt: "q".into(),
            ingestion_system_prompt: "i".into(),
            image_processing_model: "gpt-4o-mini".into(),
            image_processing_prompt: "p".into(),
            voice_processing_model: "whisper-1".into(),
            last_index_rebuild_at: None,
            index_rebuild_lease_owner: None,
            index_rebuild_lease_expires_at: None,
        }
    }

    #[test]
    fn plan_fastembed_update_sets_dimensions_from_model_metadata() {
        let current = SystemSettings {
            embedding_backend: Some(EmbeddingBackend::FastEmbed),
            embedding_model: "Xenova/bge-small-en-v1.5".into(),
            embedding_dimensions: 384,
            ..openai_settings()
        };
        let input = ModelSettingsInput {
            query_model: current.query_model.clone(),
            processing_model: current.processing_model.clone(),
            image_processing_model: current.image_processing_model.clone(),
            voice_processing_model: current.voice_processing_model.clone(),
            embedding_model: Some("Xenova/bge-base-en-v1.5".into()),
            embedding_dimensions: None,
        };
        let plan =
            plan_embedding_settings_update(&current, &input, &AppConfig::default()).expect("plan");
        assert_eq!(plan.embedding_model, "Xenova/bge-base-en-v1.5");
        assert_eq!(plan.embedding_dimensions, 768);
        assert!(plan.reembedding_needed);
        assert!(plan.restart_needed);
    }

    #[test]
    fn plan_fastembed_ignores_form_when_config_overrides_model() {
        let current = SystemSettings {
            embedding_backend: Some(EmbeddingBackend::FastEmbed),
            ..openai_settings()
        };
        let input = ModelSettingsInput {
            query_model: current.query_model.clone(),
            processing_model: current.processing_model.clone(),
            image_processing_model: current.image_processing_model.clone(),
            voice_processing_model: current.voice_processing_model.clone(),
            embedding_model: Some("Xenova/bge-large-en-v1.5".into()),
            embedding_dimensions: None,
        };
        let config = AppConfig {
            embedding_backend: EmbeddingBackend::FastEmbed,
            fastembed_model: Some("Xenova/bge-small-en-v1.5".into()),
            ..AppConfig::default()
        };
        let plan = plan_embedding_settings_update(&current, &input, &config).expect("plan");
        assert_eq!(plan.embedding_model, current.embedding_model);
        assert!(!plan.restart_needed);
    }
}

#[derive(Serialize)]
pub struct SystemPromptEditData {
    settings: SystemSettings,
    default_query_prompt: String,
}

pub async fn show_edit_system_prompt(State(state): State<HtmlState>) -> TemplateResult {
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

pub async fn show_edit_ingestion_prompt(State(state): State<HtmlState>) -> TemplateResult {
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

pub async fn show_edit_image_prompt(State(state): State<HtmlState>) -> TemplateResult {
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
