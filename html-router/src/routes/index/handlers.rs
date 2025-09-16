use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use futures::try_join;
use serde::Serialize;
use tokio::join;

use crate::{
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
    AuthSessionType,
};
use common::storage::store;
use common::storage::types::user::DashboardStats;
use common::{
    error::AppError,
    storage::types::{
        conversation::Conversation, file_info::FileInfo, ingestion_task::IngestionTask,
        knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
        text_chunk::TextChunk, text_content::TextContent, user::User,
    },
};

use crate::html_state::HtmlState;

#[derive(Serialize)]
pub struct IndexPageData {
    user: Option<User>,
    text_contents: Vec<TextContent>,
    stats: DashboardStats,
    active_jobs: Vec<IngestionTask>,
    conversation_archive: Vec<Conversation>,
}

pub async fn index_handler(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
) -> Result<impl IntoResponse, HtmlError> {
    let Some(user) = auth.current_user else {
        return Ok(TemplateResponse::redirect("/signin"));
    };

    let (text_contents, conversation_archive, stats, active_jobs) = try_join!(
        User::get_latest_text_contents(&user.id, &state.db),
        User::get_user_conversations(&user.id, &state.db),
        User::get_dashboard_stats(&user.id, &state.db),
        User::get_unfinished_ingestion_tasks(&user.id, &state.db)
    )?;

    Ok(TemplateResponse::new_template(
        "dashboard/base.html",
        IndexPageData {
            user: Some(user),
            text_contents,
            stats,
            conversation_archive,
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
                FileInfo::delete_by_id(&file_info.id, &state.db, &state.config).await
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
        "dashboard/active_jobs.html",
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

    Ok(TemplateResponse::new_template(
        "dashboard/active_jobs.html",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
    ))
}

pub async fn serve_file(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(file_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let file_info = match FileInfo::get_by_id(&file_id, &state.db).await {
        Ok(info) => info,
        _ => return Ok(TemplateResponse::not_found().into_response()),
    };

    if file_info.user_id != user.id {
        return Ok(TemplateResponse::unauthorized().into_response());
    }

    let stream = match store::get_stream_at(&file_info.path, &state.config).await {
        Ok(s) => s,
        Err(_) => return Ok(TemplateResponse::server_error().into_response()),
    };
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&file_info.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    let Ok(disposition_value) =
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", file_info.file_name))
    else {
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
        return Ok((StatusCode::OK, headers, body).into_response());
    };
    headers.insert(header::CONTENT_DISPOSITION, disposition_value);

    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );

    Ok((StatusCode::OK, headers, body).into_response())
}
