use axum::{response::Html, Extension};
use tera::{Context, Tera};

use crate::error::ApiError;

pub async fn index_handler(Extension(tera): Extension<Tera>) -> Result<Html<String>, ApiError> {
    let output = tera.render("index.html", &Context::new()).unwrap();

    Ok(output.into())
}
