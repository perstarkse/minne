use axum::{
    extract::State,
    response::{Html, IntoResponse},
    Form,
};
use axum_htmx::HxBoosted;
use serde::{Deserialize, Serialize};

use common::storage::types::user::User;

use crate::{
    html_state::HtmlState,
    middlewares::response_middleware::{HtmlError, TemplateResponse},
    AuthSessionType,
};

#[derive(Deserialize, Serialize)]
pub struct SignupParams {
    pub email: String,
    pub password: String,
    pub timezone: String,
}

pub async fn show_signup_form(
    auth: AuthSessionType,
    HxBoosted(boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    if auth.is_authenticated() {
        return Ok(TemplateResponse::redirect("/"));
    }

    if boosted { Ok(TemplateResponse::new_partial(
        "auth/signup_form.html",
        "body",
        (),
    )) } else { Ok(TemplateResponse::new_template("auth/signup_form.html", ())) }
}

pub async fn process_signup_and_show_verification(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
    Form(form): Form<SignupParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match User::create_new(form.email, form.password, &state.db, form.timezone).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!("{:?}", e);
            return Ok(Html(format!("<p>{e}</p>")).into_response());
        }
    };

    auth.login_user(user.id);

    Ok(TemplateResponse::redirect("/").into_response())
}
