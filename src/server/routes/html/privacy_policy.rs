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

page_data!(PrivacyPolicyData, "documentation/privacy.html", {
    user: Option<User>
});

pub async fn show_privacy_policy(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let output = render_template(
        PrivacyPolicyData::template_name(),
        PrivacyPolicyData {
            user: auth.current_user,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
