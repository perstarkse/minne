use axum::response::IntoResponse;

use crate::{
    middlewares::response_middleware::{HtmlError, TemplateResponse},
    AuthSessionType,
};

pub async fn sign_out_user(auth: AuthSessionType) -> Result<impl IntoResponse, HtmlError> {
    if !auth.is_authenticated() {
        return Ok(TemplateResponse::redirect("/"));
    }

    auth.logout_user();

    Ok(TemplateResponse::redirect("/"))
}
