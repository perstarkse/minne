use async_openai::types::ListModelResponse;
use axum::{extract::State, response::IntoResponse, Form};
use serde::{Deserialize, Serialize};

use common::{
    error::AppError,
    storage::types::{
        analytics::Analytics,
        conversation::Conversation,
        knowledge_entity::KnowledgeEntity,
        system_prompts::{
            DEFAULT_IMAGE_PROCESSING_PROMPT, DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT,
            DEFAULT_QUERY_SYSTEM_PROMPT,
        },
        system_settings::SystemSettings,
        text_chunk::TextChunk,
        user::User,
    },
};
use tracing::{error, info};

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
    default_image_prompt: String,
    conversation_archive: Vec<Conversation>,
    available_models: ListModelResponse,
}

pub async fn show_admin_panel(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let (
        settings_res,
        analytics_res,
        user_count_res,
        conversation_archive_res,
        available_models_res,
    ) = tokio::join!(
        SystemSettings::get_current(&state.db),
        Analytics::get_current(&state.db),
        Analytics::get_users_amount(&state.db),
        User::get_user_conversations(&user.id, &state.db),
        async { state.openai_client.models().list().await }
    );

    Ok(TemplateResponse::new_template(
        "admin/base.html",
        AdminPanelData {
            user,
            settings: settings_res?,
            analytics: analytics_res?,
            available_models: available_models_res
                .map_err(|e| AppError::InternalError(e.to_string()))?,
            users: user_count_res?,
            default_query_prompt: DEFAULT_QUERY_SYSTEM_PROMPT.to_string(),
            default_image_prompt: DEFAULT_IMAGE_PROCESSING_PROMPT.to_string(),
            conversation_archive: conversation_archive_res?,
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
        "admin/base.html",
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
    embedding_model: String,
    embedding_dimensions: Option<u32>,
}

#[derive(Serialize)]
pub struct ModelSettingsData {
    settings: SystemSettings,
    available_models: ListModelResponse,
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

    // Determine if re-embedding is required
    let reembedding_needed = input
        .embedding_dimensions
        .is_some_and(|new_dims| new_dims != current_settings.embedding_dimensions);

    let new_settings = SystemSettings {
        query_model: input.query_model,
        processing_model: input.processing_model,
        image_processing_model: input.image_processing_model,
        voice_processing_model: input.voice_processing_model,
        embedding_model: input.embedding_model,
        // Use new dimensions if provided, otherwise retain the current ones.
        embedding_dimensions: input
            .embedding_dimensions
            .unwrap_or(current_settings.embedding_dimensions),
        ..current_settings.clone()
    };

    SystemSettings::update(&state.db, new_settings.clone()).await?;

    if reembedding_needed {
        info!("Embedding dimensions changed. Spawning background re-embedding task...");

        let db_for_task = state.db.clone();
        let openai_for_task = state.openai_client.clone();
        let new_model_for_task = new_settings.embedding_model.clone();
        let new_dims_for_task = new_settings.embedding_dimensions;

        tokio::spawn(async move {
            // First, update all text chunks
            if let Err(e) = TextChunk::update_all_embeddings(
                &db_for_task,
                &openai_for_task,
                &new_model_for_task,
                new_dims_for_task,
            )
            .await
            {
                error!("Background re-embedding task failed for TextChunks: {}", e);
            }

            // Second, update all knowledge entities
            if let Err(e) = KnowledgeEntity::update_all_embeddings(
                &db_for_task,
                &openai_for_task,
                &new_model_for_task,
                new_dims_for_task,
            )
            .await
            {
                error!(
                    "Background re-embedding task failed for KnowledgeEntities: {}",
                    e
                );
            }
        });
    }

    let available_models = state
        .openai_client
        .models()
        .list()
        .await
        .map_err(|_e| AppError::InternalError("Failed to get models".to_string()))?;

    Ok(TemplateResponse::new_partial(
        "admin/base.html",
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
        "admin/base.html",
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
        "admin/base.html",
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
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

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
    RequireUser(user): RequireUser,
    Form(input): Form<ImagePromptUpdateInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not admin
    if !user.admin {
        return Ok(TemplateResponse::redirect("/"));
    };

    let current_settings = SystemSettings::get_current(&state.db).await?;

    let new_settings = SystemSettings {
        image_processing_prompt: input.image_processing_prompt,
        ..current_settings.clone()
    };

    SystemSettings::update(&state.db, new_settings.clone()).await?;

    Ok(TemplateResponse::new_partial(
        "admin/base.html",
        "system_prompt_section",
        SystemPromptSectionData {
            settings: new_settings,
        },
    ))
}
