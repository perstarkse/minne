//! SSR + HTMX HTML router for Minne.
//!
//! Handlers return [`middlewares::response_middleware::TemplateResponse`] values;
//! the template middleware renders them with shared layout context. Route composition
//! and middleware layering are handled by [`router_factory::RouterFactory`].

// minijinja_embed output (release builds) triggers these lints.
#![allow(unused_variables, clippy::expect_used, clippy::missing_panics_doc)]

pub mod html_state;
pub mod middlewares;
pub mod router_factory;
pub mod routes;
pub mod utils;

use axum::{Router, extract::FromRef};
use axum_session::{Session, SessionStore};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use common::storage::types::user::User;
use html_state::HtmlState;
use router_factory::RouterFactory;
use surrealdb::{Surreal, engine::any::Any};

pub type AuthSessionType = AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>;
pub type SessionType = Session<SessionSurrealPool<Any>>;
pub type SessionStoreType = SessionStore<SessionSurrealPool<Any>>;
pub type OpenAIClientType = async_openai::Client<async_openai::config::OpenAIConfig>;

/// Builds the HTML router with public/protected routes, assets, and middleware.
pub fn html_routes<S>(app_state: &HtmlState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    RouterFactory::new(app_state)
        .add_public_routes(routes::auth::router())
        .with_public_assets("/assets", "assets/")
        .add_protected_routes(routes::index::protected_router())
        .add_protected_routes(routes::search::router())
        .add_protected_routes(routes::account::router())
        .add_protected_routes(routes::admin::router())
        .add_protected_routes(routes::chat::router())
        .add_protected_routes(routes::content::router())
        .add_protected_routes(routes::knowledge::router())
        .add_protected_routes(routes::ingestion::router(
            app_state.config.ingest_max_body_bytes,
        ))
        .add_protected_routes(routes::scratchpad::router())
        .with_compression()
        .build()
}
