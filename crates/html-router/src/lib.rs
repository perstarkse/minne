pub mod html_state;
pub mod middlewares;
pub mod router_factory;
pub mod routes;

use axum::{extract::FromRef, Router};
use axum_session::Session;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use common::storage::types::user::User;
use html_state::HtmlState;
use router_factory::RouterFactory;
use surrealdb::{engine::any::Any, Surreal};

pub type AuthSessionType = AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>;
pub type SessionType = Session<SessionSurrealPool<Any>>;

/// Html routes
pub fn html_routes<S>(app_state: &HtmlState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    RouterFactory::new(app_state)
        .add_public_routes(routes::index::public_router())
        .add_public_routes(routes::auth::router())
        .with_public_assets("/assets", "assets/")
        .add_protected_routes(routes::index::protected_router())
        .add_protected_routes(routes::search::router())
        .add_protected_routes(routes::account::router())
        .add_protected_routes(routes::admin::router())
        .add_protected_routes(routes::chat::router())
        .add_protected_routes(routes::content::router())
        .add_protected_routes(routes::knowledge::router())
        .add_protected_routes(routes::ingestion::router())
        .build()
}
