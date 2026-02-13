use api_state::ApiState;
use axum::{
    extract::{DefaultBodyLimit, FromRef},
    middleware::from_fn_with_state,
    routing::{get, post},
    Router,
};
use middleware_api_auth::api_auth;
use routes::{categories::get_categories, ingest::ingest_data, liveness::live, readiness::ready};

pub mod api_state;
pub mod error;
mod middleware_api_auth;
mod routes;

/// Router for API functionality, version 1
pub fn api_routes_v1<S>(app_state: &ApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ApiState: FromRef<S>,
{
    // Public, unauthenticated endpoints (for k8s/systemd probes)
    let public = Router::new()
        .route("/ready", get(ready))
        .route("/live", get(live));

    // Protected API endpoints (require auth)
    let protected = Router::new()
        .route(
            "/ingest",
            post(ingest_data).layer(DefaultBodyLimit::max(
                app_state.config.ingest_max_body_bytes,
            )),
        )
        .route("/categories", get(get_categories))
        .route_layer(from_fn_with_state(app_state.clone(), api_auth));

    public.merge(protected)
}
