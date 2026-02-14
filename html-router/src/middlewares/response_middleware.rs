use std::collections::HashMap;

use axum::{
    extract::{Request, State},
    http::{HeaderName, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    Extension,
};
use axum_htmx::{HxRequest, HX_TRIGGER};
use common::{
    error::AppError,
    utils::template_engine::{ProvidesTemplateEngine, Value},
};
use minijinja::context;
use serde::Serialize;
use serde_json::json;
use tracing::error;

use crate::{html_state::HtmlState, AuthSessionType};
use common::storage::types::{
    conversation::Conversation,
    user::{Theme, User},
};

pub trait ProvidesHtmlState {
    fn html_state(&self) -> &HtmlState;
}

#[derive(Clone, Debug)]
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

#[derive(Serialize)]
struct TemplateUser {
    id: String,
    email: String,
    admin: bool,
    timezone: String,
    theme: String,
}

impl From<&User> for TemplateUser {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            admin: user.admin,
            timezone: user.timezone.clone(),
            theme: user.theme.as_str().to_string(),
        }
    }
}

#[derive(Serialize)]
struct ContextWrapper<'a> {
    user_theme: &'a str,
    initial_theme: &'a str,
    is_authenticated: bool,
    user: Option<&'a TemplateUser>,
    conversation_archive: Vec<Conversation>,
    #[serde(flatten)]
    context: HashMap<String, Value>,
}

pub async fn with_template_response<S>(
    State(state): State<S>,
    HxRequest(is_htmx): HxRequest,
    req: Request,
    next: Next,
) -> Response
where
    S: ProvidesTemplateEngine + ProvidesHtmlState + Clone + Send + Sync + 'static,
{
    let mut user_theme = Theme::System.as_str();
    let mut initial_theme = Theme::System.initial_theme();
    let mut is_authenticated = false;
    let mut current_user_id = None;
    let mut current_user = None;

    {
        if let Some(auth) = req.extensions().get::<AuthSessionType>() {
            if let Some(user) = &auth.current_user {
                is_authenticated = true;
                current_user_id = Some(user.id.clone());
                user_theme = user.theme.as_str();
                initial_theme = user.theme.initial_theme();
                current_user = Some(TemplateUser::from(user));
            }
        }
    }

    let response = next.run(req).await;

    // Headers to forward from the original response
    const HTMX_HEADERS_TO_FORWARD: &[&str] = &["HX-Push", "HX-Trigger", "HX-Redirect"];

    if let Some(template_response) = response.extensions().get::<TemplateResponse>().cloned() {
        let template_engine = state.template_engine();

        let mut conversation_archive = Vec::new();

        let should_load_conversation_archive =
            matches!(&template_response.template_kind, TemplateKind::Full(_));

        if should_load_conversation_archive {
            if let Some(user_id) = current_user_id {
                let html_state = state.html_state();
                if let Some(cached_archive) =
                    html_state.get_cached_conversation_archive(&user_id).await
                {
                    conversation_archive = cached_archive;
                } else if let Ok(archive) =
                    User::get_user_conversations(&user_id, &html_state.db).await
                {
                    html_state
                        .set_cached_conversation_archive(&user_id, archive.clone())
                        .await;
                    conversation_archive = archive;
                }
            }
        }

        fn context_to_map(
            value: &Value,
        ) -> Result<HashMap<String, Value>, minijinja::value::ValueKind> {
            match value.kind() {
                minijinja::value::ValueKind::Map => {
                    let mut map = HashMap::new();
                    if let Ok(keys) = value.try_iter() {
                        for key in keys {
                            if let Ok(val) = value.get_item(&key) {
                                map.insert(key.to_string(), val);
                            }
                        }
                    }
                    Ok(map)
                }
                minijinja::value::ValueKind::None | minijinja::value::ValueKind::Undefined => {
                    Ok(HashMap::new())
                }
                other => Err(other),
            }
        }

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

        let context_map = match context_to_map(&template_response.context) {
            Ok(map) => map,
            Err(kind) => {
                error!(
                    "Template context must be a map or unit, got kind={:?} for template_kind={:?}",
                    kind, template_response.template_kind
                );
                return (StatusCode::INTERNAL_SERVER_ERROR, Html(fallback_error())).into_response();
            }
        };

        let context = ContextWrapper {
            user_theme: &user_theme,
            initial_theme: &initial_theme,
            is_authenticated,
            user: current_user.as_ref(),
            conversation_archive,
            context: context_map,
        };

        match &template_response.template_kind {
            TemplateKind::Full(name) => {
                match template_engine.render(name, &Value::from_serialize(&context)) {
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
                match template_engine.render_block(
                    template,
                    block,
                    &Value::from_serialize(&context),
                ) {
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
                        r#"{"toast":{"title":"Error","description":"An unexpected error occurred.", "type":"error"}}"#
                            .to_string()
                    });
                    (StatusCode::NO_CONTENT, [(HX_TRIGGER, trigger_value)], "").into_response()
                } else {
                    // Non-HTMX request: Render the full errors/error.html page
                    match template_engine
                        .render("errors/error.html", &Value::from_serialize(&context))
                    {
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
