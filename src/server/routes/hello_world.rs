use axum::Extension;
use tera::{Context, Tera};

use crate::error::ApiError;

pub async fn hello_world_handler(Extension(tera): Extension<Tera>) -> Result<String, ApiError> {
    let output = tera.render("hello_world.html", &Context::new()).unwrap();

    Ok(output)
}
