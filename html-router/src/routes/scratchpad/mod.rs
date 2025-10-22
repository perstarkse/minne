mod handlers;
use axum::{
    extract::FromRef,
    routing::{delete, get, patch, post},
    Router,
};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/scratchpad", get(handlers::show_scratchpad_page))
        .route("/scratchpad", post(handlers::create_scratchpad))
        .route(
            "/scratchpad/{id}/modal",
            get(handlers::show_scratchpad_modal),
        )
        .route(
            "/scratchpad/{id}/auto-save",
            patch(handlers::auto_save_scratchpad),
        )
        .route(
            "/scratchpad/{id}/title",
            patch(handlers::update_scratchpad_title),
        )
        .route("/scratchpad/{id}", delete(handlers::delete_scratchpad))
        .route(
            "/scratchpad/{id}/archive",
            post(handlers::archive_scratchpad),
        )
        .route("/scratchpad/{id}/ingest", post(handlers::ingest_scratchpad))
        .route(
            "/scratchpad/{id}/restore",
            post(handlers::restore_scratchpad),
        )
}
