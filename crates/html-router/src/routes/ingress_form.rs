use axum::{
    extract::State,
    response::{Html, IntoResponse},
};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use futures::{future::try_join_all, TryFutureExt};
use serde::Serialize;
use tempfile::NamedTempFile;
use tracing::info;

use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
        user::User,
    },
};

use crate::{
    html_state::HtmlState,
    middleware_auth::RequireUser,
    routes::index::ActiveJobsData,
    template_response::{HtmlError, TemplateResponse},
};

pub async fn show_ingress_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let user_categories = User::get_user_categories(&user.id, &state.db).await?;

    #[derive(Serialize)]
    pub struct ShowIngressFormData {
        user_categories: Vec<String>,
    }

    Ok(TemplateResponse::new_template(
        "index/signed_in/ingress_modal.html",
        ShowIngressFormData { user_categories },
    ))
}

pub async fn hide_ingress_form(
    RequireUser(_user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
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

pub async fn process_ingress_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    TypedMultipart(input): TypedMultipart<IngressParams>,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    pub struct IngressFormData {
        instructions: String,
        content: String,
        category: String,
        error: String,
    }

    if input.content.as_ref().map_or(true, |c| c.len() < 2) && input.files.is_empty() {
        return Ok(TemplateResponse::new_template(
            "index/signed_in/ingress_form.html",
            IngressFormData {
                instructions: input.instructions.clone(),
                content: input.content.clone().unwrap_or_default(),
                category: input.category.clone(),
                error: "You need to either add files or content".to_string(),
            },
        ));
    }

    info!("{:?}", input);

    let file_infos = try_join_all(
        input
            .files
            .into_iter()
            .map(|file| FileInfo::new(file, &state.db, &user.id).map_err(|e| AppError::from(e))),
    )
    .await?;

    let payloads = IngestionPayload::create_ingestion_payload(
        input.content,
        input.instructions,
        input.category,
        file_infos,
        user.id.as_str(),
    )?;

    let futures: Vec<_> = payloads
        .into_iter()
        .map(|object| {
            IngestionTask::create_and_add_to_db(object.clone(), user.id.clone(), &state.db)
        })
        .collect();

    try_join_all(futures).await?;

    // Update the active jobs page with the newly created job
    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "index/signed_in/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
    ))
}
