use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect},
};
use axum_htmx::HxRedirect;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::{analytics::Analytics, system_settings::SystemSettings, user::User},
};

page_data!(AdminPanelData, "auth/admin_panel.html", {
    user: User,
    settings: SystemSettings,
    analytics: Analytics
});

pub async fn show_admin_panel(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let settings = SystemSettings::get_current(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;
    let analytics = Analytics::get_current(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        AdminPanelData::template_name(),
        AdminPanelData {
            user,
            settings,
            analytics,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
