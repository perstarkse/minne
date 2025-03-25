use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_session_surreal::SessionSurrealPool;
use surrealdb::engine::any::Any;

use common::storage::types::analytics::Analytics;

use crate::html_state::HtmlState;

pub async fn analytics_middleware(
    State(state): State<HtmlState>,
    session: axum_session::Session<SessionSurrealPool<Any>>,
    request: Request,
    next: Next,
) -> Response {
    // Get the path from the request
    let path = request.uri().path();

    // Only count if it's a main page request (not assets or other resources)
    if !path.starts_with("/assets") && !path.starts_with("/_next") && !path.contains('.') {
        if !session.get::<bool>("counted_visitor").unwrap_or(false) {
            let _ = Analytics::increment_visitors(&state.db).await;
            session.set("counted_visitor", true);
        }

        let _ = Analytics::increment_page_loads(&state.db).await;
    }

    next.run(request).await
}
