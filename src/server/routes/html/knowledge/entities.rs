use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_session::Session;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tokio::join;
use tracing::info;

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{
        routes::html::{render_block, render_template},
        AppState,
    },
    storage::{
        db::{delete_item, get_item},
        types::{
            file_info::FileInfo, job::Job, knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
            text_content::TextContent, user::User,
        },
    },
};

page_data!(KnowledgeBaseData, "knowledge/base.html", {
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
    user: User
});

pub async fn show_knowledge_page(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let entities = User::get_knowledge_entities(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    info!("Got entities ok");

    let relationships = User::get_knowledge_relationships(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        KnowledgeBaseData::template_name(),
        KnowledgeBaseData {
            entities,
            relationships,
            user,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}
