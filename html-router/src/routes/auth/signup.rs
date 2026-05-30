use axum::{extract::State, Form};
use axum_htmx::HxBoosted;
use serde::{Deserialize, Serialize};

use common::{error::AppError, storage::types::user::{Theme, User}};

use crate::{
    html_state::HtmlState,
    middlewares::response_middleware::{TemplateResponse, TemplateResult},
    AuthSessionType,
};

#[derive(Deserialize, Serialize)]
pub struct Params {
    pub email: String,
    pub password: String,
    pub timezone: String,
}

fn signup_error_message(err: &AppError) -> &str {
    match err {
        AppError::Auth(message) if message == "Registration is not allowed" => message,
        _ => "Could not create account. Please try again.",
    }
}

pub async fn show_signup_form(
    auth: AuthSessionType,
    HxBoosted(boosted): HxBoosted,
) -> TemplateResult {
    if auth.current_user.is_some() {
        return Ok(TemplateResponse::redirect("/"));
    }

    if boosted {
        Ok(TemplateResponse::new_partial(
            "auth/signup_form.html",
            "body",
            (),
        ))
    } else {
        Ok(TemplateResponse::new_template("auth/signup_form.html", ()))
    }
}

pub async fn process_signup_and_show_verification(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
    Form(form): Form<Params>,
) -> TemplateResult {
    let user = match User::create_new(
        form.email,
        form.password,
        &state.db,
        form.timezone,
        Theme::System.as_str().to_string(),
    )
    .await
    {
        Ok(user) => user,
        Err(err) => {
            tracing::error!(?err, "signup failed");
            return Ok(TemplateResponse::bad_request(signup_error_message(&err)));
        }
    };

    auth.login_user(user.id);

    Ok(TemplateResponse::redirect("/"))
}
