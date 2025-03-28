use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

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
    if !path.starts_with("/assets") && !path.contains('.') {
        if !session.get::<bool>("counted_visitor").unwrap_or(false) {
            let _ = Analytics::increment_visitors(state.db()).await;
            session.set("counted_visitor", true);
        }
        let _ = Analytics::increment_page_loads(state.db()).await;
    }
    next.run(request).await
}
