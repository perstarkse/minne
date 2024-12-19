use std::sync::Arc;

use axum::response::Html;
use minijinja_autoreload::AutoReloader;

pub mod auth;
pub mod index;
pub mod search_result;

pub fn render_template<T>(
    template_name: &str,
    context: T,
    templates: Arc<AutoReloader>,
) -> Result<Html<String>, minijinja::Error>
where
    T: serde::Serialize,
{
    let env = templates.acquire_env()?;
    let tmpl = env.get_template(template_name)?;

    let context = minijinja::Value::from_serialize(&context);
    let output = tmpl.render(context)?;

    Ok(output.into())
}

pub fn render_block<T>(
    template_name: &str,
    block: &str,
    context: T,
    templates: Arc<AutoReloader>,
) -> Result<Html<String>, minijinja::Error>
where
    T: serde::Serialize,
{
    let env = templates.acquire_env()?;
    let tmpl = env.get_template(template_name)?;

    let context = minijinja::Value::from_serialize(&context);
    let output = tmpl.eval_to_state(context)?.render_block(block)?;

    Ok(output.into())
}

#[macro_export]
macro_rules! page_data {
    ($name:ident, {$($(#[$attr:meta])* $field:ident: $ty:ty),*$(,)?}) => {
        use serde::{Serialize, Deserialize};

        #[derive(Debug, Deserialize, Serialize)]
        pub struct $name {
            $($(#[$attr])* pub $field: $ty),*
        }
    };
}
