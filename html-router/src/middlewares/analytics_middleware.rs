use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::Response,
};
use tracing::warn;

use common::storage::{db::ProvidesDb, types::analytics::Analytics};

use crate::SessionType;

/// Middleware to count unique visitors and page loads
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
    // Only count visits/page loads for GET requests to non-asset, non-static paths
    if request.method() == Method::GET && !path.starts_with("/assets") && !path.contains('.') {
        if !session.get::<bool>("counted_visitor").unwrap_or(false) {
            if let Err(e) = Analytics::increment_visitors(state.db()).await {
                warn!("failed to increment visitor count: {e}");
            }
            session.set("counted_visitor", true);
        }
        if let Err(e) = Analytics::increment_page_loads(state.db()).await {
            warn!("failed to increment page load count: {e}");
        }
    }
    next.run(request).await
}
