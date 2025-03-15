use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Serialize;

use common::{
    error::AppError,
    storage::types::{knowledge_entity::KnowledgeEntity, user::User},
};

use crate::{
    html_state::HtmlState,
    middleware_auth::RequireUser,
    template_response::{HtmlError, TemplateResponse},
};

pub async fn show_reference_tooltip(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(reference_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let entity: KnowledgeEntity = state
        .db
        .get_item(&reference_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Item was not found".to_string()))?;

    if entity.user_id != user.id {
        return Ok(TemplateResponse::unauthorized());
    }

    #[derive(Serialize)]
    struct ReferenceTooltipData {
        entity: KnowledgeEntity,
        user: User,
    }

    Ok(TemplateResponse::new_template(
        "chat/reference_tooltip.html",
        ReferenceTooltipData { entity, user },
    ))
}
