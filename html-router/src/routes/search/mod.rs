mod handlers;

use axum::{extract::FromRef, routing::get, Router};
#[allow(clippy::module_name_repetitions)]
pub use handlers::{
    search_result_handler as result_handler, SearchParams as SearchQueryParams,
};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new().route("/search", get(result_handler))
}
