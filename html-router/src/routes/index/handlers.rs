use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Serialize;
use tokio::join;

use crate::{
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
    AuthSessionType,
};
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_task::IngestionTask, knowledge_entity::KnowledgeEntity,
        knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
        text_content::TextContent, user::User,
    },
};

use crate::html_state::HtmlState;

#[derive(Serialize)]
pub struct IndexPageData {
    user: Option<User>,
    latest_text_contents: Vec<TextContent>,
    active_jobs: Vec<IngestionTask>,
}

pub async fn index_handler(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
) -> Result<impl IntoResponse, HtmlError> {
    let Some(user) = auth.current_user else {
        return Ok(TemplateResponse::new_template(
            "index/index.html",
            IndexPageData {
                user: None,
                latest_text_contents: vec![],
                active_jobs: vec![],
            },
        ));
    };

    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db).await?;

    let latest_text_contents = User::get_latest_text_contents(user.id.as_str(), &state.db).await?;

    Ok(TemplateResponse::new_template(
        "index/index.html",
        IndexPageData {
            user: Some(user),
            latest_text_contents,
            active_jobs,
        },
    ))
}

#[derive(Serialize)]
pub struct LatestTextContentData {
    latest_text_contents: Vec<TextContent>,
    user: User,
}

pub async fn delete_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Get and validate TextContent
    let text_content = get_and_validate_text_content(&state, &id, &user).await?;

    // Perform concurrent deletions
    let (_res1, _res2, _res3, _res4, _res5) = join!(
        async {
            if let Some(file_info) = text_content.file_info {
                FileInfo::delete_by_id(&file_info.id, &state.db).await
            } else {
                Ok(())
            }
        },
        state.db.delete_item::<TextContent>(&text_content.id),
        TextChunk::delete_by_source_id(&text_content.id, &state.db),
        KnowledgeEntity::delete_by_source_id(&text_content.id, &state.db),
        KnowledgeRelationship::delete_relationships_by_source_id(&text_content.id, &state.db)
    );

    // Render updated content
    let latest_text_contents = User::get_latest_text_contents(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "index/signed_in/recent_content.html",
        "latest_content_section",
        LatestTextContentData {
            user: user.to_owned(),
            latest_text_contents,
        },
    ))
}

// Helper function to get and validate text content
async fn get_and_validate_text_content(
    state: &HtmlState,
    id: &str,
    user: &User,
) -> Result<TextContent, AppError> {
    let text_content = state
        .db
        .get_item::<TextContent>(id)
        .await?
        .ok_or_else(|| AppError::NotFound("Item was not found".to_string()))?;

    if text_content.user_id != user.id {
        return Err(AppError::Auth(
            "You are not the owner of that content".to_string(),
        ));
    }

    Ok(text_content)
}

#[derive(Serialize)]
pub struct ActiveJobsData {
    pub active_jobs: Vec<IngestionTask>,
    pub user: User,
}

pub async fn delete_job(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    User::validate_and_delete_job(&id, &user.id, &state.db).await?;

    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "index/signed_in/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
    ))
}

pub async fn show_active_jobs(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    return Ok(TemplateResponse::server_error());
    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "index/signed_in/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
    ))
}
