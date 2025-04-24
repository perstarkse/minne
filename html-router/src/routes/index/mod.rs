pub mod handlers;

use axum::{
    extract::FromRef,
    routing::{delete, get},
    Router,
};
use handlers::{delete_job, delete_text_content, index_handler, show_active_jobs};

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
        .route("/active-jobs", get(show_active_jobs))
        .route("/text-content/{id}", delete(delete_text_content))
}
