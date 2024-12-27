use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect},
    Form,
};
use axum_htmx::{HxBoosted, HxRedirect};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};

use crate::{error::ApiError, server::AppState, storage::types::user::User};

use super::{render_block, render_template};

#[derive(Deserialize, Serialize)]
pub struct SignupParams {
    pub email: String,
    pub password: String,
    pub remember_me: Option<String>,
}

#[derive(Serialize)]
struct PageData {
    // name: String,
}

pub async fn show_signin_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    HxBoosted(boosted): HxBoosted,
) -> Result<impl IntoResponse, ApiError> {
    if auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }
    let output = match boosted {
        true => render_block(
            "auth/signin_form.html",
            "body",
            PageData {},
            state.templates,
        )?,
        false => render_template("auth/signin_form.html", PageData {}, state.templates)?,
    };

    Ok(output.into_response())
}

pub async fn authenticate_user(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<SignupParams>,
) -> Result<impl IntoResponse, ApiError> {
    let user = User::authenticate(form.email, form.password, &state.surreal_db_client).await?;
    auth.login_user(user.id);
    if form
        .remember_me
        .is_some_and(|string| string == "on".to_string())
    {
        auth.remember_user(true);
    }
    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}
