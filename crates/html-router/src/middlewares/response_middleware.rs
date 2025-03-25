use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    Extension,
};
use common::error::AppError;
use minijinja::{context, Value};
use minijinja_autoreload::AutoReloader;
use serde::Serialize;
use std::sync::Arc;

use crate::html_state::HtmlState;

// Enum for template types
#[derive(Clone)]
pub enum TemplateKind {
    Full(String),            // Full page template
    Partial(String, String), // Template name, block name
    Error(StatusCode),       // Error template with status code
    Redirect(String),        // Redirect
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

    pub fn error(status: StatusCode, title: &str, error: &str, description: &str) -> Self {
        let ctx = context! {
            status_code => status.as_u16(),
            title => title,
            error => error,
            description => description
        };

        Self {
            template_kind: TemplateKind::Error(status),
            context: ctx,
        }
    }

    // Convenience methods for common errors
    pub fn not_found() -> Self {
        Self::error(
            StatusCode::NOT_FOUND,
            "Page Not Found",
            "Not Found",
            "The page you're looking for doesn't exist or was removed.",
        )
    }

    pub fn server_error() -> Self {
        Self::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            "Internal Server Error",
            "Something went wrong on our end.",
        )
    }

    pub fn unauthorized() -> Self {
        Self::error(
            StatusCode::UNAUTHORIZED,
            "Unauthorized",
            "Access Denied",
            "You need to be logged in to access this page.",
        )
    }

    pub fn bad_request(message: &str) -> Self {
        Self::error(
            StatusCode::BAD_REQUEST,
            "Bad Request",
            "Bad Request",
            message,
        )
    }

    pub fn redirect(path: impl Into<String>) -> Self {
        Self {
            template_kind: TemplateKind::Redirect(path.into()),
            context: Value::from_serialize(&()),
        }
    }
}

impl IntoResponse for TemplateResponse {
    fn into_response(self) -> Response {
        Extension(self).into_response()
    }
}

struct TemplateStateWrapper {
    state: HtmlState,
    template_response: TemplateResponse,
}

impl IntoResponse for TemplateStateWrapper {
    fn into_response(self) -> Response {
        let templates = self.state.templates;

        match &self.template_response.template_kind {
            TemplateKind::Full(name) => {
                render_template(name, self.template_response.context, templates)
            }
            TemplateKind::Partial(name, block) => {
                render_block(name, block, self.template_response.context, templates)
            }
            TemplateKind::Error(status) => {
                let html = match try_render_template(
                    "errors/error.html",
                    self.template_response.context,
                    templates,
                ) {
                    Ok(html_string) => Html(html_string),
                    Err(_) => fallback_error(),
                };
                (*status, html).into_response()
            }
            TemplateKind::Redirect(path) => {
                (StatusCode::OK, [(axum_htmx::HX_REDIRECT, path.clone())], "").into_response()
            }
        }
    }
}

// Helper functions for rendering with error handling
fn render_template(name: &str, context: Value, templates: Arc<AutoReloader>) -> Response {
    match try_render_template(name, context, templates.clone()) {
        Ok(html) => Html(html).into_response(),
        Err(_) => fallback_error().into_response(),
    }
}

fn render_block(name: &str, block: &str, context: Value, templates: Arc<AutoReloader>) -> Response {
    match try_render_block(name, block, context, templates.clone()) {
        Ok(html) => Html(html).into_response(),
        Err(_) => fallback_error().into_response(),
    }
}

fn try_render_template(
    template_name: &str,
    context: Value,
    templates: Arc<AutoReloader>,
) -> Result<String, ()> {
    let env = templates.acquire_env().map_err(|e| {
        tracing::error!("Environment error: {:?}", e);
        ()
    })?;

    let tmpl = env.get_template(template_name).map_err(|e| {
        tracing::error!("Template error: {:?}", e);
        ()
    })?;

    tmpl.render(context).map_err(|e| {
        tracing::error!("Render error: {:?}", e);
        ()
    })
}

fn try_render_block(
    template_name: &str,
    block: &str,
    context: Value,
    templates: Arc<AutoReloader>,
) -> Result<String, ()> {
    let env = templates.acquire_env().map_err(|e| {
        tracing::error!("Environment error: {:?}", e);
        ()
    })?;

    let tmpl = env.get_template(template_name).map_err(|e| {
        tracing::error!("Template error: {:?}", e);
        ()
    })?;

    let mut state = tmpl.eval_to_state(context).map_err(|e| {
        tracing::error!("Eval error: {:?}", e);
        ()
    })?;

    state.render_block(block).map_err(|e| {
        tracing::error!("Block render error: {:?}", e);
        ()
    })
}

fn fallback_error() -> Html<String> {
    Html(
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
        .to_string(),
    )
}

pub async fn with_template_response(
    State(state): State<HtmlState>,
    response: Response,
) -> Response {
    // Clone the TemplateResponse from extensions
    let template_response = response.extensions().get::<TemplateResponse>().cloned();

    if let Some(template_response) = template_response {
        TemplateStateWrapper {
            state,
            template_response,
        }
        .into_response()
    } else {
        response
    }
}

// Define HtmlError
pub enum HtmlError {
    AppError(AppError),
}

// Conversion from AppError to HtmlError
impl From<AppError> for HtmlError {
    fn from(err: AppError) -> Self {
        HtmlError::AppError(err)
    }
}

// Conversion for database error to HtmlError
impl From<surrealdb::Error> for HtmlError {
    fn from(err: surrealdb::Error) -> Self {
        HtmlError::AppError(AppError::from(err))
    }
}

impl IntoResponse for HtmlError {
    fn into_response(self) -> Response {
        match self {
            HtmlError::AppError(err) => {
                let template_response = match err {
                    AppError::NotFound(_) => TemplateResponse::not_found(),
                    AppError::Auth(_) => TemplateResponse::unauthorized(),
                    AppError::Validation(msg) => TemplateResponse::bad_request(&msg),
                    _ => {
                        tracing::error!("Internal error: {:?}", err);
                        TemplateResponse::server_error()
                    }
                };
                template_response.into_response()
            }
        }
    }
}
