use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
    Form,
};
use axum_htmx::{HxBoosted, HxRedirect};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use common::{error::HtmlError, storage::types::user::User};

use crate::{html_state::HtmlState, page_data};

use super::{render_block, render_template};

#[derive(Deserialize, Serialize)]
pub struct SignupParams {
    pub email: String,
    pub password: String,
    pub remember_me: Option<String>,
}

page_data!(ShowSignInForm, "auth/signin_form.html", {});

pub async fn show_signin_form(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    HxBoosted(boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    if auth.is_authenticated() {
        return Ok(Redirect::to("/").into_response());
    }
    let output = match boosted {
        true => render_block(
            ShowSignInForm::template_name(),
            "body",
            ShowSignInForm {},
            state.templates.clone(),
        )?,
        false => render_template(
            ShowSignInForm::template_name(),
            ShowSignInForm {},
            state.templates.clone(),
        )?,
    };

    Ok(output.into_response())
}

pub async fn authenticate_user(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<SignupParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match User::authenticate(form.email, form.password, &state.db).await {
        Ok(user) => user,
        Err(_) => {
            return Ok(Html("<p>Incorrect email or password </p>").into_response());
        }
    };

    auth.login_user(user.id);

    if form.remember_me.is_some_and(|string| string == *"on") {
        auth.remember_user(true);
    }

    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}
