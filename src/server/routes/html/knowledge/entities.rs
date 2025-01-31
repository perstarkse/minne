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

page_data!(KnowledgeEntitiesData, "todo", {
    gdpr_accepted: bool,
    user: Option<User>,
    latest_text_contents: Vec<TextContent>,
    active_jobs: Vec<Job>
});
pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    session: Session<SessionSurrealPool<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    Ok("Hi".into_response())
}
