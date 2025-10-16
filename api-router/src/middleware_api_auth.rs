use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use common::storage::types::user::User;

use crate::{api_state::ApiState, error::ApiError};

pub async fn api_auth(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let api_key = extract_api_key(&request)
        .ok_or_else(|| ApiError::Unauthorized("You have to be authenticated".to_string()))?;

    let user = User::find_by_api_key(&api_key, &state.db).await?;
    let user =
        user.ok_or_else(|| ApiError::Unauthorized("You have to be authenticated".to_string()))?;

    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}

fn extract_api_key(request: &Request) -> Option<String> {
    request
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            request
                .headers()
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|auth| auth.strip_prefix("Bearer ").map(str::trim))
        })
        .map(String::from)
}
