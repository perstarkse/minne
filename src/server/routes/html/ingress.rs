use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use futures::{future::try_join_all, TryFutureExt};
use serde::Serialize;
use surrealdb::{engine::any::Any, Surreal};
use tempfile::NamedTempFile;
use tracing::info;

use crate::{
    error::{AppError, HtmlError},
    ingress::types::ingress_input::{create_ingress_objects, IngressInput},
    server::AppState,
    storage::types::{file_info::FileInfo, user::User},
};

use super::render_template;

#[derive(Serialize)]
struct PageData {
    // name: String,
}

pub async fn show_ingress_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    let output = render_template("ingress_form.html", PageData {}, state.templates.clone())
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

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

pub async fn process_ingress_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    TypedMultipart(input): TypedMultipart<IngressParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

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

    Ok(Html("SuccessBRO!").into_response())
}
