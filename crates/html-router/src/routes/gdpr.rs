use axum::response::{Html, IntoResponse};

use crate::SessionType;

pub async fn accept_gdpr(session: SessionType) -> impl IntoResponse {
    session.set("gdpr_accepted", true);

    Html("").into_response()
}

pub async fn deny_gdpr(session: SessionType) -> impl IntoResponse {
    session.set("gdpr_accepted", true);

    Html("").into_response()
}
