use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use common::storage::types::user::User;

use crate::{api_state::ApiState, error::ApiErr};

pub async fn api_auth(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiErr> {
    let api_key = extract_api_key(&request)
        .ok_or_else(|| ApiErr::Unauthorized("You have to be authenticated".to_string()))?;

    let user = User::find_by_api_key(api_key, &state.db).await?;
    let user =
        user.ok_or_else(|| ApiErr::Unauthorized("You have to be authenticated".to_string()))?;

    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}

fn extract_api_key(request: &Request) -> Option<&str> {
    request
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            request
                .headers()
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|auth| auth.strip_prefix("Bearer "))
                .map(str::trim)
        })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use axum::body::Body;
    use axum::http::{HeaderValue, Request};

    use super::extract_api_key;

    fn request_with_headers(headers: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder().method("GET").uri("/");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).expect("test request")
    }

    #[test]
    fn extract_api_key_from_x_api_key_header() {
        let request = request_with_headers(&[("X-API-Key", "sk_test_key")]);
        assert_eq!(extract_api_key(&request), Some("sk_test_key"));
    }

    #[test]
    fn extract_api_key_from_bearer_authorization() {
        let request = request_with_headers(&[("Authorization", "Bearer sk_bearer_key")]);
        assert_eq!(extract_api_key(&request), Some("sk_bearer_key"));
    }

    #[test]
    fn extract_api_key_prefers_x_api_key_over_authorization() {
        let request = request_with_headers(&[
            ("X-API-Key", "sk_header"),
            ("Authorization", "Bearer sk_bearer"),
        ]);
        assert_eq!(extract_api_key(&request), Some("sk_header"));
    }

    #[test]
    fn extract_api_key_returns_none_when_missing() {
        let request = request_with_headers(&[]);
        assert_eq!(extract_api_key(&request), None);
    }

    #[test]
    fn extract_api_key_rejects_non_bearer_authorization() {
        let request = request_with_headers(&[("Authorization", "Basic abc")]);
        assert_eq!(extract_api_key(&request), None);
    }

    #[test]
    fn extract_api_key_rejects_invalid_header_values() {
        let mut request = request_with_headers(&[]);
        request.headers_mut().insert(
            "X-API-Key",
            HeaderValue::from_bytes(&[0xFF]).expect("invalid header"),
        );
        assert_eq!(extract_api_key(&request), None);
    }
}
