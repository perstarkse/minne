mod handlers;

use axum::{extract::DefaultBodyLimit, extract::FromRef, routing::get, Router};
use handlers::{get_task_updates_stream, hide_ingest_form, process_ingest_form, show_ingest_form};

use crate::html_state::HtmlState;

pub fn router<S>(max_body_bytes: usize) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route(
            "/ingest-form",
            get(show_ingest_form)
                .post(process_ingest_form)
                .layer(DefaultBodyLimit::max(max_body_bytes)),
        )
        .route("/task/status-stream", get(get_task_updates_stream))
        .route("/hide-ingest-form", get(hide_ingest_form))
}
