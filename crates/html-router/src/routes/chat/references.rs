use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::Serialize;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::routes::HtmlError;
use common::{
    error::AppError,
    storage::types::{knowledge_entity::KnowledgeEntity, user::User},
};

use crate::{html_state::HtmlState, routes::render_template};

pub async fn show_reference_tooltip(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(reference_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Showing reference");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let entity: KnowledgeEntity = state
        .db
        .get_item(&reference_id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?
        .ok_or_else(|| {
            HtmlError::new(
                AppError::NotFound("Item was not found".to_string()),
                state.templates.clone(),
            )
        })?;

    if entity.user_id != user.id {
        return Err(HtmlError::new(
            AppError::Auth("You dont have access to this entity".to_string()),
            state.templates.clone(),
        ));
    }

    #[derive(Serialize)]
    struct ReferenceTooltipData {
        entity: KnowledgeEntity,
        user: User,
    }

    let output = render_template(
        "chat/reference_tooltip.html",
        ReferenceTooltipData { entity, user },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
