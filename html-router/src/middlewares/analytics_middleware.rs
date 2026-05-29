use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::Response,
};
use tracing::warn;

use common::storage::{db::ProvidesDb, types::analytics::Analytics};

use crate::SessionType;

/// Middleware to count unique visitors and page loads.
pub async fn analytics_middleware<S>(
    State(state): State<S>,
    session: SessionType,
    request: Request,
    next: Next,
) -> Response
where
    S: ProvidesDb + Clone + Send + Sync + 'static,
{
    let path = request.uri().path();
    if request.method() == Method::GET && !path.starts_with("/assets") && !path.contains('.') {
        let is_new_visitor = !session.get::<bool>("counted_visitor").unwrap_or(false);
        if is_new_visitor {
            session.set("counted_visitor", true);
        }

        let db = Arc::clone(state.db());
        tokio::spawn(async move {
            if let Err(error) = Analytics::record_page_view(&db, is_new_visitor).await {
                warn!("failed to record page view: {error}");
            }
        });
    }
    next.run(request).await
}
