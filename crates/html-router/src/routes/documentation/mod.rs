use axum::response::IntoResponse;
use common::storage::types::user::User;
use serde::Serialize;

use crate::template_response::{HtmlError, TemplateResponse};
use crate::AuthSessionType;

#[derive(Serialize)]
pub struct DocumentationPageData {
    user: Option<User>,
    current_path: String,
}

pub async fn show_privacy_policy(auth: AuthSessionType) -> Result<impl IntoResponse, HtmlError> {
    Ok(TemplateResponse::new_template(
        "documentation/privacy.html",
        DocumentationPageData {
            user: auth.current_user,
            current_path: "/privacy-policy".to_string(),
        },
    ))
}

pub async fn show_get_started(auth: AuthSessionType) -> Result<impl IntoResponse, HtmlError> {
    Ok(TemplateResponse::new_template(
        "documentation/get_started.html",
        DocumentationPageData {
            user: auth.current_user,
            current_path: "/get-started".to_string(),
        },
    ))
}

pub async fn show_mobile_friendly(auth: AuthSessionType) -> Result<impl IntoResponse, HtmlError> {
    Ok(TemplateResponse::new_template(
        "documentation/mobile_friendly.html",
        DocumentationPageData {
            user: auth.current_user,
            current_path: "/mobile-friendly".to_string(),
        },
    ))
}

pub async fn show_documentation_index(
    auth: AuthSessionType,
) -> Result<impl IntoResponse, HtmlError> {
    Ok(TemplateResponse::new_template(
        "documentation/index.html",
        DocumentationPageData {
            user: auth.current_user,
            current_path: "/index".to_string(),
        },
    ))
}
