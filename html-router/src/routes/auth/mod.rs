pub mod signin;
pub mod signout;
pub mod signup;

use axum::{extract::FromRef, routing::get, Router};
use signin::{authenticate_user, show_signin_form};
use signout::sign_out_user;
use signup::{process_signup_and_show_verification, show_signup_form};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/signout", get(sign_out_user))
        .route("/signin", get(show_signin_form).post(authenticate_user))
        .route(
            "/signup",
            get(show_signup_form).post(process_signup_and_show_verification),
        )
}
