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
        .route("/account", get(handlers::show_account_page))
        .route("/set-api-key", post(handlers::set_api_key))
        .route("/update-timezone", patch(handlers::update_timezone))
        .route(
            "/change-password",
            get(handlers::show_change_password).patch(handlers::change_password),
        )
        .route("/delete-account", delete(handlers::delete_account))
}
