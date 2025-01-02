use axum::response::{Html, IntoResponse};
use axum_session::Session;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::engine::any::Any;

use crate::error::HtmlError;

pub async fn accept_gdpr(
    session: Session<SessionSurrealPool<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    session.set("gdpr_accepted", true);

    Ok(Html("").into_response())
}

pub async fn deny_gdpr(
    session: Session<SessionSurrealPool<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    session.set("gdpr_accepted", true);

    Ok(Html("").into_response())
}
