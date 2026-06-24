mod handlers;

use axum::{Router, extract::FromRef, routing::get};
#[allow(clippy::module_name_repetitions)]
pub use handlers::{SearchParams as SearchQueryParams, search_result_handler as result_handler};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new().route("/search", get(result_handler))
}
