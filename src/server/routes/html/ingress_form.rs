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

use crate::{
    error::{AppError, HtmlError, IntoHtmlError},
    ingress::types::ingress_input::{create_ingress_objects, IngressInput},
    page_data,
    server::AppState,
    storage::types::{file_info::FileInfo, user::User},
};

use super::render_template;

pub async fn show_ingress_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    let output = render_template("ingress_form.html", {}, state.templates.clone())?;

    Ok(output.into_response())
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
    State(state): State<AppState>,
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
        FileInfo::new(file, &state.surreal_db_client, &user.id)
            .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))
    }))
    .await?;

    let ingress_objects = create_ingress_objects(
        IngressInput {
            content: input.content,
            instructions: input.instructions,
            category: input.category,
            files: file_infos,
        },
        user.id.as_str(),
    )
    .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let futures: Vec<_> = ingress_objects
        .into_iter()
        .map(|object| state.job_queue.enqueue(object.clone(), user.id.clone()))
        .collect();

    try_join_all(futures)
        .await
        .map_err(AppError::from)
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    Ok(Html(
        "<a class='btn btn-primary' hx-get='/ingress-form' hx-swap='outerHTML'>Add Content</a>",
    )
    .into_response())
}
