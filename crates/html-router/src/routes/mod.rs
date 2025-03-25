use std::sync::Arc;

use axum::response::Html;
use minijinja_autoreload::AutoReloader;

use crate::middlewares::response_middleware::HtmlError;

pub mod account;
pub mod admin;
pub mod auth;
pub mod chat;
pub mod content;
pub mod index;
pub mod ingestion;
pub mod knowledge;
pub mod search;

// Helper function for render_template
pub fn render_template<T>(
    template_name: &str,
    context: T,
    templates: Arc<AutoReloader>,
) -> Result<Html<String>, HtmlError>
where
    T: serde::Serialize,
{
    let env = templates.acquire_env().unwrap();
    let tmpl = env.get_template(template_name).unwrap();
    let context = minijinja::Value::from_serialize(&context);
    let output = tmpl.render(context).unwrap();

    Ok(Html(output))
}
