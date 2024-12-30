use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
    Form,
};
use axum_htmx::{HxBoosted, HxRedirect};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};
use tempfile::NamedTempFile;
use tracing::info;

use crate::{
    error::ApiError,
    server::AppState,
    storage::types::{file_info::FileInfo, user::User},
};

use super::{render_block, render_template};

#[derive(Serialize)]
struct PageData {
    // name: String,
}

pub async fn show_ingress_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }

    Ok(render_template("ingress_form.html", PageData {}, state.templates)?.into_response())
}
#[derive(Debug, TryFromMultipart)]
pub struct IngressParams {
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,
    #[form_data(limit = "10000000")] // Adjust limit as needed
    pub files: Vec<FieldData<NamedTempFile>>,
}

pub async fn process_ingress_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    TypedMultipart(input): TypedMultipart<IngressParams>,
) -> Result<impl IntoResponse, ApiError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    info!("{:?}", input);

    // Process files and create FileInfo objects
    let mut file_infos = Vec::new();
    for file in input.files {
        let file_info = FileInfo::new(file, &state.surreal_db_client).await?;
        file_infos.push(file_info);
    }

    // Process the ingress (implement your logic here)

    Ok(Html("SuccessBRO!").into_response())
    // Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}
