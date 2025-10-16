use axum::{
    extract::State,
    http::{HeaderName, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Extension,
};
use axum_htmx::{HxRequest, HX_TRIGGER};
use common::{error::AppError, utils::template_engine::ProvidesTemplateEngine};
use minijinja::{context, Value};
use serde::Serialize;
use serde_json::json;
use tracing::error;

#[derive(Clone)]
pub enum TemplateKind {
    Full(String),
    Partial(String, String),
    Error(StatusCode),
    Redirect(String),
}

#[derive(Clone)]
pub struct TemplateResponse {
    template_kind: TemplateKind,
    context: Value,
}

impl TemplateResponse {
    pub fn new_template<T: Serialize>(name: impl Into<String>, context: T) -> Self {
        Self {
            template_kind: TemplateKind::Full(name.into()),
            context: Value::from_serialize(&context),
        }
    }

    pub fn new_partial<T: Serialize>(
        template: impl Into<String>,
        block: impl Into<String>,
        context: T,
    ) -> Self {
        Self {
            template_kind: TemplateKind::Partial(template.into(), block.into()),
            context: Value::from_serialize(&context),
        }
    }

    pub fn error(status: StatusCode, title: &str, description: &str) -> Self {
        let ctx = context! {
            status_code => status.as_u16(),
            title => title,
            description => description
        };
        Self {
            template_kind: TemplateKind::Error(status),
            context: ctx,
        }
    }

    pub fn not_found() -> Self {
        Self::error(
            StatusCode::NOT_FOUND,
            "Page Not Found",
            "The page you're looking for doesn't exist or was removed.",
        )
    }

    pub fn server_error() -> Self {
        Self::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            "Something went wrong on our end.",
        )
    }

    pub fn unauthorized() -> Self {
        Self::error(
            StatusCode::UNAUTHORIZED,
            "Unauthorized",
            "You need to be logged in to access this page.",
        )
    }

    pub fn bad_request(message: &str) -> Self {
        Self::error(StatusCode::BAD_REQUEST, "Bad Request", message)
    }

    pub fn redirect(path: impl Into<String>) -> Self {
        Self {
            template_kind: TemplateKind::Redirect(path.into()),
            context: Value::from_serialize(()),
        }
    }
}

impl IntoResponse for TemplateResponse {
    fn into_response(self) -> Response {
        Extension(self).into_response()
    }
}

pub async fn with_template_response<S>(
    State(state): State<S>,
    HxRequest(is_htmx): HxRequest,
    response: Response<axum::body::Body>,
) -> Response<axum::body::Body>
where
    S: ProvidesTemplateEngine + Clone + Send + Sync + 'static,
{
    // Headers to forward from the original response
    const HTMX_HEADERS_TO_FORWARD: &[&str] = &["HX-Push", "HX-Trigger", "HX-Redirect"];

    if let Some(template_response) = response.extensions().get::<TemplateResponse>().cloned() {
        let template_engine = state.template_engine();

        // Helper to forward relevant headers
        fn forward_headers(from: &axum::http::HeaderMap, to: &mut axum::http::HeaderMap) {
            for &header_name in HTMX_HEADERS_TO_FORWARD {
                if let Ok(name) = HeaderName::from_bytes(header_name.as_bytes()) {
                    if let Some(value) = from.get(&name) {
                        to.insert(name.clone(), value.clone());
                    }
                }
            }
        }

        match &template_response.template_kind {
            TemplateKind::Full(name) => {
                match template_engine.render(name, &template_response.context) {
                    Ok(html) => {
                        let mut final_response = Html(html).into_response();
                        forward_headers(response.headers(), final_response.headers_mut());
                        final_response
                    }
                    Err(e) => {
                        error!("Failed to render template '{}': {:?}", name, e);
                        (StatusCode::INTERNAL_SERVER_ERROR, Html(fallback_error())).into_response()
                    }
                }
            }
            TemplateKind::Partial(template, block) => {
                match template_engine.render_block(template, block, &template_response.context) {
                    Ok(html) => {
                        let mut final_response = Html(html).into_response();
                        forward_headers(response.headers(), final_response.headers_mut());
                        final_response
                    }
                    Err(e) => {
                        error!("Failed to render block '{}/{}': {:?}", template, block, e);
                        (StatusCode::INTERNAL_SERVER_ERROR, Html(fallback_error())).into_response()
                    }
                }
            }
            TemplateKind::Error(status) => {
                if is_htmx {
                    // HTMX request: Send 204 + HX-Trigger for toast
                    let title = template_response
                        .context
                        .get_attr("title")
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "Error".to_string());
                    let description = template_response
                        .context
                        .get_attr("description")
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "An error occurred.".to_string());

                    let trigger_payload = json!({"toast": {"title": title, "description": description, "type": "error"}});
                    let trigger_value = serde_json::to_string(&trigger_payload).unwrap_or_else(|e| {
                        error!("Failed to serialize HX-Trigger payload: {}", e);
                        r#"{"toast":{"title":"Error","description":"An unexpected error occurred.", "type":"error"}}"#.to_string()
                    });
                    (StatusCode::NO_CONTENT, [(HX_TRIGGER, trigger_value)], "").into_response()
                } else {
                    // Non-HTMX request: Render the full errors/error.html page
                    match template_engine.render("errors/error.html", &template_response.context) {
                        Ok(html) => (*status, Html(html)).into_response(),
                        Err(e) => {
                            error!("Critical: Failed to render 'errors/error.html': {:?}", e);
                            // Fallback HTML, but use the intended status code
                            (*status, Html(fallback_error())).into_response()
                        }
                    }
                }
            }
            TemplateKind::Redirect(path) => {
                if is_htmx {
                    (StatusCode::OK, [(axum_htmx::HX_REDIRECT, path)], "").into_response()
                } else {
                    Redirect::to(path).into_response()
                }
            }
        }
    } else {
        response
    }
}

#[derive(Debug)]
pub enum HtmlError {
    AppError(AppError),
    TemplateError(String),
}

impl From<AppError> for HtmlError {
    fn from(err: AppError) -> Self {
        Self::AppError(err)
    }
}

impl From<surrealdb::Error> for HtmlError {
    fn from(err: surrealdb::Error) -> Self {
        Self::AppError(AppError::from(err))
    }
}

impl From<minijinja::Error> for HtmlError {
    fn from(err: minijinja::Error) -> Self {
        Self::TemplateError(err.to_string())
    }
}

impl IntoResponse for HtmlError {
    fn into_response(self) -> Response {
        match self {
            Self::AppError(err) => match err {
                AppError::NotFound(_) => TemplateResponse::not_found().into_response(),
                AppError::Auth(_) => TemplateResponse::unauthorized().into_response(),
                AppError::Validation(msg) => TemplateResponse::bad_request(&msg).into_response(),
                _ => {
                    error!("Internal error: {:?}", err);
                    TemplateResponse::server_error().into_response()
                }
            },
            Self::TemplateError(err) => {
                error!("Template error: {}", err);
                TemplateResponse::server_error().into_response()
            }
        }
    }
}

fn fallback_error() -> String {
    r#"
    <html>
        <body>
            <div class="container mx-auto p-4">
                <h1 class="text-4xl text-error">Error</h1>
                <p class="mt-4">Sorry, something went wrong displaying this page.</p>
            </div>
        </body>
    </html>
    "#
    .to_string()
}
