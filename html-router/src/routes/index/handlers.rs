use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use futures::try_join;
use serde::Serialize;

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{
            ResponseResult, TemplateResponse, TemplateResult, template_as_response,
        },
    },
    utils::text_content_preview::truncate_text_contents,
    utils::truncate::with_ellipsis,
};
use common::storage::types::user::DashboardStats;
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_task::IngestionTask, text_content::TextContent, user::User,
    },
};

#[derive(Serialize)]
pub struct IndexPageData {
    text_contents: Vec<TextContent>,
    stats: DashboardStats,
    active_jobs: Vec<IngestionTask>,
}

pub async fn index_handler(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> TemplateResult {
    let (text_contents, dashboard_stats, active_jobs) = try_join!(
        User::get_latest_text_contents(&user.id, &state.db),
        User::get_dashboard_stats(&user.id, &state.db),
        User::get_unfinished_ingestion_tasks(&user.id, &state.db)
    )?;

    let text_contents = truncate_text_contents(text_contents);

    Ok(TemplateResponse::new_template(
        "dashboard/base.html",
        IndexPageData {
            text_contents,
            stats: dashboard_stats,
            active_jobs,
        },
    ))
}

#[derive(Serialize)]
pub struct LatestTextContentData {
    text_contents: Vec<TextContent>,
}

pub async fn delete_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> TemplateResult {
    // Get and validate TextContent
    let text_content = get_and_validate_text_content(&state, &id, &user).await?;

    // Remove stored assets before deleting the text content record
    if let Some(file_info) = text_content.file_info.as_ref() {
        let file_in_use =
            TextContent::has_other_with_file(&file_info.id, &text_content.id, &state.db).await?;

        if !file_in_use {
            FileInfo::delete_by_id_with_storage(&file_info.id, &state.db, &state.storage).await?;
        }
    }

    TextContent::clear_ingested_children(&text_content.id, &user.id, &state.db).await?;
    state
        .db
        .delete_item::<TextContent>(&text_content.id)
        .await?;

    // Render updated content
    let text_contents =
        truncate_text_contents(User::get_latest_text_contents(&user.id, &state.db).await?);

    Ok(TemplateResponse::new_partial(
        "dashboard/recent_content.html",
        "latest_content_section",
        LatestTextContentData { text_contents },
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
        .ok_or_else(|| AppError::NotFound("item was not found".to_string()))?;

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
}

#[derive(Serialize)]
struct TaskArchiveEntry {
    id: String,
    state_label: String,
    state_raw: String,
    attempts: u32,
    max_attempts: u32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    scheduled_at: DateTime<Utc>,
    locked_at: Option<DateTime<Utc>>,
    last_error_at: Option<DateTime<Utc>>,
    error_message: Option<String>,
    worker_id: Option<String>,
    priority: i32,
    lease_duration_secs: i64,
    content_kind: String,
    content_summary: String,
}

#[derive(Serialize)]
struct TaskArchiveData {
    tasks: Vec<TaskArchiveEntry>,
}

pub async fn delete_job(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> TemplateResult {
    User::validate_and_delete_job(&id, &user.id, &state.db).await?;

    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "dashboard/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData { active_jobs },
    ))
}

pub async fn show_active_jobs(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> TemplateResult {
    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "dashboard/active_jobs.html",
        ActiveJobsData { active_jobs },
    ))
}

pub async fn show_task_archive(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> TemplateResult {
    let tasks = User::get_all_ingestion_tasks(&user.id, &state.db).await?;

    let entries: Vec<TaskArchiveEntry> = tasks
        .into_iter()
        .map(|task| {
            let (content_kind, content_summary) = summarize_task_content(&task);

            TaskArchiveEntry {
                id: task.id.clone(),
                state_label: task.state.display_label().to_string(),
                state_raw: task.state.as_str().to_string(),
                attempts: task.attempts,
                max_attempts: task.max_attempts,
                created_at: task.created_at,
                updated_at: task.updated_at,
                scheduled_at: task.scheduled_at,
                locked_at: task.locked_at,
                last_error_at: task.last_error_at,
                error_message: task.error_message.clone(),
                worker_id: task.worker_id.clone(),
                priority: task.priority,
                lease_duration_secs: task.lease_duration_secs,
                content_kind,
                content_summary,
            }
        })
        .collect();

    Ok(TemplateResponse::new_template(
        "dashboard/task_archive_modal.html",
        TaskArchiveData { tasks: entries },
    ))
}

fn summarize_task_content(task: &IngestionTask) -> (String, String) {
    match &task.content {
        common::storage::types::ingestion_payload::IngestionPayload::Text { text, .. } => {
            ("Text".to_string(), with_ellipsis(text, 80))
        }
        common::storage::types::ingestion_payload::IngestionPayload::Url { url, .. } => {
            ("URL".to_string(), url.clone())
        }
        common::storage::types::ingestion_payload::IngestionPayload::File { file_info, .. } => {
            ("File".to_string(), file_info.file_name.clone())
        }
    }
}

pub async fn serve_file(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(file_id): Path<String>,
) -> ResponseResult {
    let Ok(file_info) = FileInfo::get_by_id(&file_id, &state.db).await else {
        return Ok(template_as_response(TemplateResponse::not_found()));
    };

    if file_info.user_id != user.id {
        return Ok(template_as_response(TemplateResponse::unauthorized()));
    }

    let Ok(stream) = state.storage.get_stream(&file_info.path).await else {
        return Ok(template_as_response(TemplateResponse::server_error()));
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
