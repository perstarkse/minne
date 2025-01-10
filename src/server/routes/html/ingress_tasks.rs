use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::AppState,
    storage::types::{
        job::{Job, JobStatus},
        user::User,
    },
};
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use super::render_template;

page_data!(ShowQueueTasks, "queue_tasks.html", {user : User,jobs: Vec<Job>});

pub async fn show_queue_tasks(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let jobs = state
        .job_queue
        .get_user_jobs(&user.id)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    for job in &jobs {
        match job.status {
            JobStatus::Created => info!("Found a created job"),
            _ => continue,
        }
    }

    let rendered = render_template(
        ShowQueueTasks::template_name(),
        ShowQueueTasks { jobs, user },
        state.templates.clone(),
    )
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    Ok(rendered.into_response())
}

pub async fn delete_task(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    state
        .job_queue
        .delete_job(&id, &user.id)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    Ok(Html("").into_response())
}
