use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Form,
};
use axum_htmx::{HxBoosted, HxRequest};
use serde::{Deserialize, Serialize};

use common::storage::types::{
    conversation::Conversation, file_info::FileInfo, text_content::TextContent, user::User,
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

#[derive(Serialize)]
pub struct ContentPageData {
    user: User,
    text_contents: Vec<TextContent>,
    categories: Vec<String>,
    selected_category: Option<String>,
    conversation_archive: Vec<Conversation>,
}

#[derive(Deserialize)]
pub struct FilterParams {
    category: Option<String>,
}

pub async fn show_content_page(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Query(params): Query<FilterParams>,
    HxRequest(is_htmx): HxRequest,
    HxBoosted(is_boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    // Normalize empty strings to None
    let has_category_param = params.category.is_some();
    let category_filter = params.category.as_deref().unwrap_or("").trim();

    // load categories and filtered/all contents
    let categories = User::get_user_categories(&user.id, &state.db).await?;
    let text_contents = if !category_filter.is_empty() {
        User::get_text_contents_by_category(&user.id, category_filter, &state.db).await?
    } else {
        User::get_text_contents(&user.id, &state.db).await?
    };

    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;
    let data = ContentPageData {
        user,
        text_contents,
        categories,
        selected_category: params.category.clone(),
        conversation_archive,
    };

    if is_htmx && !is_boosted && has_category_param {
        // If HTMX partial request with filter applied, return partial content list update
        return Ok(TemplateResponse::new_partial(
            "content/base.html",
            "main",
            data,
        ));
    }

    // Otherwise full page response including layout
    Ok(TemplateResponse::new_template("content/base.html", data))
}

pub async fn show_text_content_edit_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    #[derive(Serialize)]
    pub struct TextContentEditModal {
        pub user: User,
        pub text_content: TextContent,
    }

    Ok(TemplateResponse::new_template(
        "content/edit_text_content_modal.html",
        TextContentEditModal { user, text_content },
    ))
}

#[derive(Deserialize)]
pub struct PatchTextContentParams {
    instructions: String,
    category: String,
    text: String,
}
pub async fn patch_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
    Form(form): Form<PatchTextContentParams>,
) -> Result<impl IntoResponse, HtmlError> {
    User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    TextContent::patch(
        &id,
        &form.instructions,
        &form.category,
        &form.text,
        &state.db,
    )
    .await?;

    let text_contents = User::get_text_contents(&user.id, &state.db).await?;
    let categories = User::get_user_categories(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "content/base.html",
        "main",
        ContentPageData {
            user,
            text_contents,
            categories,
            selected_category: None,
            conversation_archive,
        },
    ))
}

pub async fn delete_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Get and validate the text content
    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    // If it has file info, delete that too
    if let Some(file_info) = &text_content.file_info {
        FileInfo::delete_by_id(&file_info.id, &state.db).await?;
    }

    // Delete the text content
    state.db.delete_item::<TextContent>(&id).await?;

    // Get updated content, categories and return the refreshed list
    let text_contents = User::get_text_contents(&user.id, &state.db).await?;
    let categories = User::get_user_categories(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "content/content_list.html",
        ContentPageData {
            user,
            text_contents,
            categories,
            selected_category: None,
            conversation_archive,
        },
    ))
}

pub async fn show_content_read_modal(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Get and validate the text content
    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db).await?;
    #[derive(Serialize)]
    pub struct TextContentReadModalData {
        pub user: User,
        pub text_content: TextContent,
    }

    Ok(TemplateResponse::new_template(
        "content/read_content_modal.html",
        TextContentReadModalData { user, text_content },
    ))
}
