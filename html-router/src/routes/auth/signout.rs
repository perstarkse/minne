use crate::{
    AuthSessionType,
    middlewares::response_middleware::{TemplateResponse, TemplateResult},
};

pub async fn sign_out_user(auth: AuthSessionType) -> TemplateResult {
    if !auth.is_authenticated() {
        return Ok(TemplateResponse::redirect("/"));
    }

    auth.logout_user();

    Ok(TemplateResponse::redirect("/"))
}
