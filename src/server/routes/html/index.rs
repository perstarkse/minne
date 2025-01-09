use axum::{extract::State, response::IntoResponse};
use axum_session::Session;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::user::User,
};

page_data!(IndexData, "index/index.html", {
    gdpr_accepted: bool,
    queue_length: u32,
    user: Option<User>
});

pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    session: Session<SessionSurrealPool<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying index page");

    let gdpr_accepted = auth.current_user.is_some() | session.get("gdpr_accepted").unwrap_or(false);

    let queue_length = match auth.current_user.is_some() {
        true => state
            .job_queue
            .get_user_jobs(&auth.current_user.clone().unwrap().id)
            .await
            .map_err(|e| HtmlError::new(e, state.templates.clone()))?
            .len(),
        false => 0,
    };

    // let knowledge_entities = User::get_knowledge_entities(
    //     &auth.current_user.clone().unwrap().id,
    //     &state.surreal_db_client,
    // )
    // .await?;

    // info!("{:?}", knowledge_entities);

    let output = render_template(
        IndexData::template_name(),
        IndexData {
            queue_length: queue_length.try_into().unwrap(),
            gdpr_accepted,
            user: auth.current_user,
        },
        state.templates.clone(),
    )
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    Ok(output.into_response())
}
