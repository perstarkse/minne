use axum::{extract::State, response::IntoResponse};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    error::HtmlError,
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::user::User,
};

page_data!(DocumentationData, "do_not_use_this", {
    user: Option<User>,
    current_path: String
});

pub async fn show_privacy_policy(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let output = render_template(
        "documentation/privacy.html",
        DocumentationData {
            user: auth.current_user,
            current_path: "/privacy_policy".to_string(),
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn show_get_started(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let output = render_template(
        "documentation/get_started.html",
        DocumentationData {
            user: auth.current_user,
            current_path: "/get-started".to_string(),
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
pub async fn show_mobile_friendly(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let output = render_template(
        "documentation/mobile_friendly.html",
        DocumentationData {
            user: auth.current_user,
            current_path: "/mobile-friendly".to_string(),
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn show_documentation_index(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let output = render_template(
        "documentation/index.html",
        DocumentationData {
            user: auth.current_user,
            current_path: "/index".to_string(),
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
