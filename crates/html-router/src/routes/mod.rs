use std::sync::Arc;

use axum::response::Html;
use minijinja_autoreload::AutoReloader;

use crate::template_response::HtmlError;

pub mod account;
pub mod admin_panel;
pub mod chat;
pub mod content;
pub mod documentation;
pub mod gdpr;
pub mod index;
pub mod ingress_form;
pub mod knowledge;
pub mod search_result;
pub mod signin;
pub mod signout;
pub mod signup;

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
