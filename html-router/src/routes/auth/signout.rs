use crate::{
    middlewares::response_middleware::{TemplateResponse, TemplateResult},
    AuthSessionType,
};

pub async fn sign_out_user(auth: AuthSessionType) -> TemplateResult {
    if !auth.is_authenticated() {
        return Ok(TemplateResponse::redirect("/"));
    }

    auth.logout_user();

    Ok(TemplateResponse::redirect("/"))
}
