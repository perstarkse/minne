mod handlers;

use axum::{extract::FromRef, routing::get, Router};
use handlers::{
    get_task_updates_stream, hide_ingress_form, process_ingress_form, show_ingress_form,
};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route(
            "/ingress-form",
            get(show_ingress_form).post(process_ingress_form),
        )
        .route("/task/status-stream", get(get_task_updates_stream))
        .route("/hide-ingress-form", get(hide_ingress_form))
}
