use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use futures::{future::try_join_all, TryFutureExt};
use surrealdb::{engine::any::Any, Surreal};
use tempfile::NamedTempFile;
use tracing::info;

use common::{
    error::{AppError, HtmlError, IntoHtmlError},
    storage::types::{
        file_info::FileInfo, ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
        user::User,
    },
};

use crate::{
    html_state::HtmlState,
    page_data,
    routes::{index::ActiveJobsData, render_block},
};

use super::render_template;

#[derive(Serialize)]
pub struct ShowIngressFormData {
    user_categories: Vec<String>,
}

pub async fn show_ingress_form(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    let user_categories = User::get_user_categories(&auth.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        "index/signed_in/ingress_modal.html",
        ShowIngressFormData { user_categories },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn hide_ingress_form(
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    Ok(Html(
        "<a class='btn btn-primary' hx-get='/ingress-form' hx-swap='outerHTML'>Add Content</a>",
    )
    .into_response())
}

#[derive(Debug, TryFromMultipart)]
pub struct IngressParams {
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,
    #[form_data(limit = "10000000")] // Adjust limit as needed
    #[form_data(default)]
    pub files: Vec<FieldData<NamedTempFile>>,
}

page_data!(IngressFormData, "ingress_form.html", {
    instructions: String,
    content: String,
    category: String,
    error: String,
});

pub async fn process_ingress_form(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    TypedMultipart(input): TypedMultipart<IngressParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = auth.current_user.ok_or_else(|| {
        AppError::Auth("You must be signed in".to_string()).with_template(state.templates.clone())
    })?;

    if input.content.clone().is_some_and(|c| c.len() < 2) && input.files.is_empty() {
        let output = render_template(
            IngressFormData::template_name(),
            IngressFormData {
                instructions: input.instructions.clone(),
                content: input.content.clone().unwrap(),
                category: input.category.clone(),
                error: "You need to either add files or content".to_string(),
            },
            state.templates.clone(),
        )?;

        return Ok(output.into_response());
    }

    info!("{:?}", input);

    let file_infos = try_join_all(input.files.into_iter().map(|file| {
        FileInfo::new(file, &state.db, &user.id)
            .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))
    }))
    .await?;

    let payloads = IngestionPayload::create_ingestion_payload(
        input.content,
        input.instructions,
        input.category,
        file_infos,
        user.id.as_str(),
    )
    .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let futures: Vec<_> = payloads
        .into_iter()
        .map(|object| {
            IngestionTask::create_and_add_to_db(object.clone(), user.id.clone(), &state.db)
        })
        .collect();

    try_join_all(futures)
        .await
        .map_err(AppError::from)
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Update the active jobs page with the newly created job
    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_block(
        "index/signed_in/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
