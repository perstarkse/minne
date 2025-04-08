use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    Extension,
};
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

pub async fn with_template_response<S>(State(state): State<S>, response: Response) -> Response
where
    S: ProvidesTemplateEngine + Clone + Send + Sync + 'static,
{
    if let Some(template_response) = response.extensions().get::<TemplateResponse>().cloned() {
        let template_engine = state.template_engine();

        match &template_response.template_kind {
            TemplateKind::Full(name) => {
                match template_engine.render(name, &template_response.context) {
                    Ok(html) => Html(html).into_response(),
                    Err(e) => {
                        error!("Failed to render template '{}': {:?}", name, e);
                        (StatusCode::INTERNAL_SERVER_ERROR, Html(fallback_error())).into_response()
                    }
                }
            }
            TemplateKind::Partial(template, block) => {
                match template_engine.render_block(template, block, &template_response.context) {
                    Ok(html) => Html(html).into_response(),
                    Err(e) => {
                        error!("Failed to render block '{}/{}': {:?}", template, block, e);
                        (StatusCode::INTERNAL_SERVER_ERROR, Html(fallback_error())).into_response()
                    }
                }
            }
            TemplateKind::Error(_status) => {
                // Extract title and description from context
                let title = template_response
                    .context
                    .get_attr("title")
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Error".to_string()); // Fallback title
                let description = template_response
                    .context
                    .get_attr("description")
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "An error occurred.".to_string()); // Fallback desc

                let trigger_payload = json!({
                    "toast": {
                        "title": title,
                        "description": description,
                        "type": "error"
                    }
                });

                // Convert payload to string
                let trigger_value = serde_json::to_string(&trigger_payload)
                    .unwrap_or_else(|e| {
                        error!("Failed to serialize HX-Trigger payload: {}", e);
                        // Fallback trigger if serialization fails
                        r#"{"toast":{"title":"Error","description":"An unexpected error occurred.", "type":"error"}}"#.to_string()
                    });

                // Return 204 No Content with HX-Trigger header
                (
                    StatusCode::NO_CONTENT,
                    [(axum_htmx::HX_TRIGGER, trigger_value)],
                    "", // Empty body for 204
                )
                    .into_response()
            }
            TemplateKind::Redirect(path) => {
                (StatusCode::OK, [(axum_htmx::HX_REDIRECT, path.clone())], "").into_response()
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
        HtmlError::AppError(err)
    }
}

impl From<surrealdb::Error> for HtmlError {
    fn from(err: surrealdb::Error) -> Self {
        HtmlError::AppError(AppError::from(err))
    }
}

impl From<minijinja::Error> for HtmlError {
    fn from(err: minijinja::Error) -> Self {
        HtmlError::TemplateError(err.to_string())
    }
}

impl IntoResponse for HtmlError {
    fn into_response(self) -> Response {
        match self {
            HtmlError::AppError(err) => match err {
                AppError::NotFound(_) => TemplateResponse::not_found().into_response(),
                AppError::Auth(_) => TemplateResponse::unauthorized().into_response(),
                AppError::Validation(msg) => TemplateResponse::bad_request(&msg).into_response(),
                _ => {
                    error!("Internal error: {:?}", err);
                    TemplateResponse::server_error().into_response()
                }
            },
            HtmlError::TemplateError(err) => {
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
