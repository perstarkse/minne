pub mod handlers;

use axum::{
    extract::FromRef,
    routing::{delete, get},
    Router,
};
use handlers::{
    delete_job, delete_text_content, index_handler, serve_file, show_active_jobs, show_task_archive,
};

use crate::html_state::HtmlState;

pub fn public_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new().route("/", get(index_handler))
}

pub fn protected_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/jobs/{job_id}", delete(delete_job))
        .route("/jobs/archive", get(show_task_archive))
        .route("/active-jobs", get(show_active_jobs))
        .route("/text-content/{id}", delete(delete_text_content))
        .route("/file/{id}", get(serve_file))
}
