use axum::{
    debug_handler,
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Serialize;
use tokio::join;

use crate::{
    middleware_auth::RequireUser,
    template_response::{HtmlError, TemplateResponse},
    AuthSessionType, SessionType,
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
    gdpr_accepted: bool,
    user: Option<User>,
    latest_text_contents: Vec<TextContent>,
    active_jobs: Vec<IngestionTask>,
}

pub async fn index_handler(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
    session: SessionType,
) -> Result<impl IntoResponse, HtmlError> {
    let gdpr_accepted = auth.current_user.is_some() | session.get("gdpr_accepted").unwrap_or(false);

    let active_jobs = match auth.current_user.is_some() {
        true => {
            User::get_unfinished_ingestion_tasks(&auth.current_user.clone().unwrap().id, &state.db)
                .await?
        }
        false => vec![],
    };

    let latest_text_contents = match auth.current_user.clone().is_some() {
        true => {
            User::get_latest_text_contents(
                auth.current_user.clone().unwrap().id.as_str(),
                &state.db,
            )
            .await?
        }
        false => vec![],
    };

    Ok(TemplateResponse::new_template(
        "index/index.html",
        IndexPageData {
            gdpr_accepted,
            user: auth.current_user,
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

#[debug_handler]
pub async fn delete_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Get and validate TextContent
    let text_content = get_and_validate_text_content(&state, &id, &user).await?;

    // Perform concurrent deletions
    join!(
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
