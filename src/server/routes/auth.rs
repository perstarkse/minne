use axum::{
    extract::State,
    response::{Html, IntoResponse},
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};

use crate::{error::ApiError, server::AppState, storage::types::user::User};

#[derive(Deserialize, Serialize)]
pub struct SignupParams {
    pub email: String,
    pub password: String,
}

pub async fn show_signup_form(State(state): State<AppState>) -> Html<String> {
    let context = tera::Context::new();
    let html = state
        .tera
        .render("auth/signup.html", &context)
        .unwrap_or_else(|_| "<h1>Error rendering template</h1>".to_string());
    Html(html)
}

pub async fn signup_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<SignupParams>,
) -> Result<impl IntoResponse, ApiError> {
    let user = User::create_new(form.email, form.password, &state.surreal_db_client).await?;
    auth.login_user(user.id);
    Ok(())
}
