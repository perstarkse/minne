use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
    Form,
};
use axum_htmx::{HxBoosted, HxRedirect};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};

use common::{error::HtmlError, storage::types::user::User};

use crate::html_state::HtmlState;

use super::{render_block, render_template};

#[derive(Deserialize, Serialize)]
pub struct SignupParams {
    pub email: String,
    pub password: String,
    pub timezone: String,
}

pub async fn show_signup_form(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    HxBoosted(boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    if auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }
    let output = match boosted {
        true => render_block("auth/signup_form.html", "body", {}, state.templates.clone())?,
        false => render_template("auth/signup_form.html", {}, state.templates.clone())?,
    };

    Ok(output.into_response())
}

pub async fn process_signup_and_show_verification(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<SignupParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match User::create_new(
        form.email,
        form.password,
        &state.surreal_db_client,
        form.timezone,
    )
    .await
    {
        Ok(user) => user,
        Err(e) => {
            tracing::error!("{:?}", e);
            return Ok(Html(format!("<p>{}</p>", e)).into_response());
        }
    };

    auth.login_user(user.id);

    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}
