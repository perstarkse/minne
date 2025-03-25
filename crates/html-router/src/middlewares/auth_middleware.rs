use axum::{
    async_trait,
    extract::{FromRequestParts, Request},
    http::request::Parts,
    middleware::Next,
    response::{IntoResponse, Response},
};
use common::storage::types::user::User;

use crate::AuthSessionType;

use super::response_middleware::TemplateResponse;

#[derive(Debug, Clone)]
pub struct RequireUser(pub User);

// Implement FromRequestParts for RequireUser
#[async_trait]
impl<S> FromRequestParts<S> for RequireUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<User>()
            .cloned()
            .map(RequireUser)
            .ok_or_else(|| TemplateResponse::redirect("/signin").into_response())
    }
}

// Auth middleware that adds the user to extensions
pub async fn require_auth(auth: AuthSessionType, mut request: Request, next: Next) -> Response {
    // Check if user is authenticated
    match auth.current_user {
        Some(user) => {
            // Add user to request extensions
            request.extensions_mut().insert(user);
            // Continue to the handler
            next.run(request).await
        }
        None => {
            // Redirect to login
            TemplateResponse::redirect("/signin").into_response()
        }
    }
}
