use api_state::ApiState;
use axum::{
    extract::{DefaultBodyLimit, FromRef},
    middleware::from_fn_with_state,
    routing::post,
    Router,
};
use middleware_api_auth::api_auth;
use routes::ingress::ingress_data;

pub mod api_state;
mod middleware_api_auth;
mod routes;

/// Router for API functionality, version 1
pub fn api_routes_v1<S>(app_state: &ApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ApiState: FromRef<S>,
{
    Router::new()
        .route("/ingress", post(ingress_data))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        .route_layer(from_fn_with_state(app_state.clone(), api_auth))
}
