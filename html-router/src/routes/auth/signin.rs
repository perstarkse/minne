use axum::{extract::State, response::IntoResponse, Form};
use axum_htmx::HxBoosted;
use serde::{Deserialize, Serialize};

use crate::{
    html_state::HtmlState,
    middlewares::response_middleware::{HtmlError, TemplateResponse},
    AuthSessionType,
};
use common::storage::types::user::User;

#[derive(Deserialize, Serialize)]
pub struct SignupParams {
    pub email: String,
    pub password: String,
    pub remember_me: Option<String>,
}

pub async fn show_signin_form(
    auth: AuthSessionType,
    HxBoosted(boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    if auth.is_authenticated() {
        return Ok(TemplateResponse::redirect("/"));
    }
    if boosted {
        Ok(TemplateResponse::new_partial(
            "auth/signin_base.html",
            "body",
            (),
        ))
    } else {
        Ok(TemplateResponse::new_template("auth/signin_base.html", ()))
    }
}

pub async fn authenticate_user(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
    Form(form): Form<SignupParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match User::authenticate(&form.email, &form.password, &state.db).await {
        Ok(user) => user,
        Err(_) => {
            return Ok(TemplateResponse::bad_request("Incorrect email or password").into_response());
        }
    };

    auth.login_user(user.id);

    if form.remember_me.is_some_and(|string| string == *"on") {
        auth.remember_user(true);
    }

    Ok(TemplateResponse::redirect("/").into_response())
}
