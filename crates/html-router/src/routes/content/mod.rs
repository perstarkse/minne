use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use common::storage::types::{text_content::TextContent, user::User};

use crate::{error::HtmlError, html_state::HtmlState, page_data};

use super::render_template;

page_data!(ContentPageData, "content/base.html", {
    user: User,
    text_contents: Vec<TextContent>
});

pub async fn show_content_page(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let text_contents = User::get_text_contents(&user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        ContentPageData::template_name(),
        ContentPageData {
            user,
            text_contents,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}

#[derive(Serialize)]
pub struct TextContentEditModal {
    pub user: User,
    pub text_content: TextContent,
}

pub async fn show_text_content_edit_form(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        "content/edit_text_content_modal.html",
        TextContentEditModal { user, text_content },
        state.templates,
    )?;

    Ok(output.into_response())
}

pub async fn patch_text_content(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // ADD FUNCTION TO PATCH CONTENT

    let text_contents = User::get_text_contents(&user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        "content/content_list.html",
        ContentPageData {
            user,
            text_contents,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}
