use std::sync::Arc;

use axum::response::Html;
use minijinja_autoreload::AutoReloader;

use crate::error::{HtmlError, IntoHtmlError};

pub mod account;
pub mod admin_panel;
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

pub trait PageData {
    fn template_name() -> &'static str;
}

// Helper function for render_template
pub fn render_template<T>(
    template_name: &str,
    context: T,
    templates: Arc<AutoReloader>,
) -> Result<Html<String>, HtmlError>
where
    T: serde::Serialize,
{
    let env = templates
        .acquire_env()
        .map_err(|e| e.with_template(templates.clone()))?;
    let tmpl = env
        .get_template(template_name)
        .map_err(|e| e.with_template(templates.clone()))?;
    let context = minijinja::Value::from_serialize(&context);
    let output = tmpl
        .render(context)
        .map_err(|e| e.with_template(templates.clone()))?;
    Ok(Html(output))
}

pub fn render_block<T>(
    template_name: &str,
    block: &str,
    context: T,
    templates: Arc<AutoReloader>,
) -> Result<Html<String>, HtmlError>
where
    T: serde::Serialize,
{
    let env = templates
        .acquire_env()
        .map_err(|e| e.with_template(templates.clone()))?;
    let tmpl = env
        .get_template(template_name)
        .map_err(|e| e.with_template(templates.clone()))?;

    let context = minijinja::Value::from_serialize(&context);
    let output = tmpl
        .eval_to_state(context)
        .map_err(|e| e.with_template(templates.clone()))?
        .render_block(block)
        .map_err(|e| e.with_template(templates.clone()))?;

    Ok(output.into())
}

#[macro_export]
macro_rules! page_data {
    ($name:ident, $template_name:expr, {$($(#[$attr:meta])* $field:ident: $ty:ty),*$(,)?}) => {
        use serde::{Serialize, Deserialize};
        use $crate::server::routes::html::PageData;

        #[derive(Debug, Deserialize, Serialize)]
        pub struct $name {
            $($(#[$attr])* pub $field: $ty),*
        }

        impl PageData for $name {
            fn template_name() -> &'static str {
                $template_name
            }
        }
    };
}
