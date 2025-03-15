use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Serialize;

use common::storage::types::{text_content::TextContent, user::User};

use crate::{
    html_state::HtmlState,
    middleware_auth::RequireUser,
    template_response::{HtmlError, TemplateResponse},
};

#[derive(Serialize)]
pub struct ContentPageData {
    user: User,
    text_contents: Vec<TextContent>,
}

pub async fn show_content_page(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let text_contents = User::get_text_contents(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "content/base.html",
        ContentPageData {
            user,
            text_contents,
        },
    ))
}

#[derive(Serialize)]
pub struct TextContentEditModal {
    pub user: User,
    pub text_content: TextContent,
}

pub async fn show_text_content_edit_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "content/edit_text_content_modal.html",
        TextContentEditModal { user, text_content },
    ))
}

pub async fn patch_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    // ADD FUNCTION TO PATCH CONTENT

    let text_contents = User::get_text_contents(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "content/content_list.html",
        ContentPageData {
            user,
            text_contents,
        },
    ))
}
