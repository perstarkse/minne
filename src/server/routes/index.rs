use axum::{response::Html, Extension};
use serde_json::json;
use tera::{Context, Tera};

use crate::error::ApiError;

pub async fn index_handler(Extension(tera): Extension<Tera>) -> Result<Html<String>, ApiError> {
    let output = tera
        .render(
            "index.html",
            &Context::from_value(json!({"adjective": "CRAYCRAY"})).unwrap(),
        )
        .unwrap();

    Ok(output.into())
}
