mod handlers;

use axum::{extract::FromRef, routing::get, Router};
pub use handlers::{search_result_handler, SearchParams};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new().route("/search", get(search_result_handler))
}
