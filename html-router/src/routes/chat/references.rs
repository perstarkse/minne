#![allow(clippy::missing_docs_in_private_items)]

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::Serialize;

use common::storage::types::{
    knowledge_entity::KnowledgeEntity, text_chunk::TextChunk, user::User,
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

use super::reference_validation::{normalize_reference, ReferenceLookupTarget};

#[derive(Serialize)]
struct ReferenceTooltipData {
    text_chunk: Option<TextChunk>,
    text_chunk_updated_at: Option<String>,
    entity: Option<KnowledgeEntity>,
    entity_updated_at: Option<String>,
    user: User,
}

fn format_datetime_for_user(datetime: DateTime<Utc>, timezone: &str) -> String {
    match timezone.parse::<Tz>() {
        Ok(tz) => datetime
            .with_timezone(&tz)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

pub async fn show_reference_tooltip(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(reference_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let Ok((normalized_reference_id, target)) = normalize_reference(&reference_id) else {
        return Ok(TemplateResponse::not_found());
    };

    let lookup_order = match target {
        ReferenceLookupTarget::TextChunk | ReferenceLookupTarget::Any => [
            ReferenceLookupTarget::TextChunk,
            ReferenceLookupTarget::KnowledgeEntity,
        ],
        ReferenceLookupTarget::KnowledgeEntity => [
            ReferenceLookupTarget::KnowledgeEntity,
            ReferenceLookupTarget::TextChunk,
        ],
    };

    let mut text_chunk: Option<TextChunk> = None;
    let mut knowledge_entity: Option<KnowledgeEntity> = None;

    for lookup_target in lookup_order {
        match lookup_target {
            ReferenceLookupTarget::TextChunk => {
                if let Some(chunk) = state
                    .db
                    .get_item::<TextChunk>(&normalized_reference_id)
                    .await?
                {
                    if chunk.user_id != user.id {
                        return Ok(TemplateResponse::unauthorized());
                    }
                    text_chunk = Some(chunk);
                    break;
                }
            }
            ReferenceLookupTarget::KnowledgeEntity => {
                if let Some(entity) = state
                    .db
                    .get_item::<KnowledgeEntity>(&normalized_reference_id)
                    .await?
                {
                    if entity.user_id != user.id {
                        return Ok(TemplateResponse::unauthorized());
                    }
                    knowledge_entity = Some(entity);
                    break;
                }
            }
            ReferenceLookupTarget::Any => {}
        }
    }

    if text_chunk.is_none() && knowledge_entity.is_none() {
        return Ok(TemplateResponse::not_found());
    }

    let text_chunk_updated_at = text_chunk
        .as_ref()
        .map(|chunk| format_datetime_for_user(chunk.updated_at, &user.timezone));
    let entity_updated_at = knowledge_entity
        .as_ref()
        .map(|entity| format_datetime_for_user(entity.updated_at, &user.timezone));

    Ok(TemplateResponse::new_template(
        "chat/reference_tooltip.html",
        ReferenceTooltipData {
            text_chunk,
            text_chunk_updated_at,
            entity: knowledge_entity,
            entity_updated_at,
            user,
        },
    ))
}
