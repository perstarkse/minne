use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Form,
};
use axum_htmx::{HxBoosted, HxRequest, HxTarget};
use serde::{Deserialize, Serialize};

use common::storage::types::{
    file_info::FileInfo, knowledge_entity::KnowledgeEntity, text_chunk::TextChunk,
    text_content::TextContent, user::User,
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
    utils::pagination::{paginate_items, Pagination},
    utils::text_content_preview::truncate_text_contents,
};
use url::form_urlencoded;

const CONTENTS_PER_PAGE: usize = 12;

#[derive(Serialize)]
pub struct ContentPageData {
    text_contents: Vec<TextContent>,
    categories: Vec<String>,
    selected_category: Option<String>,
    pagination: Pagination,
    page_query: String,
}

#[derive(Serialize)]
pub struct RecentTextContentData {
    pub text_contents: Vec<TextContent>,
}

#[derive(Deserialize)]
pub struct FilterParams {
    category: Option<String>,
    page: Option<usize>,
}

pub async fn show_content_page(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Query(params): Query<FilterParams>,
    HxRequest(is_htmx): HxRequest,
    HxBoosted(is_boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    // Normalize empty strings to None
    let category_filter = params
        .category
        .as_ref()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty());

    // load categories and filtered/all contents
    let categories = User::get_user_categories(&user.id, &state.db).await?;
    let full_contents = match category_filter {
        Some(category) => {
            User::get_text_contents_by_category(&user.id, category, &state.db).await?
        }
        None => User::get_text_contents(&user.id, &state.db).await?,
    };

    let (page_contents, pagination) = paginate_items(full_contents, params.page, CONTENTS_PER_PAGE);
    let text_contents = truncate_text_contents(page_contents);

    let page_query = category_filter
        .map(|category| {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("category", category);
            format!("&{}", serializer.finish())
        })
        .unwrap_or_default();

    let data = ContentPageData {
        text_contents,
        categories,
        selected_category: params.category.clone(),
        pagination,
        page_query,
    };

    if is_htmx && !is_boosted {
        Ok(TemplateResponse::new_partial(
            "content/base.html",
            "main",
            data,
        ))
    } else {
        Ok(TemplateResponse::new_template("content/base.html", data))
    }
}

pub async fn show_text_content_edit_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let text_content = User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    #[derive(Serialize)]
    pub struct TextContentEditModal {
        pub text_content: TextContent,
    }

    Ok(TemplateResponse::new_template(
        "content/edit_text_content_modal.html",
        TextContentEditModal { text_content },
    ))
}

#[derive(Deserialize)]
pub struct PatchTextContentParams {
    context: String,
    category: String,
    text: String,
}
pub async fn patch_text_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
    HxTarget(target): HxTarget,
    Form(form): Form<PatchTextContentParams>,
) -> Result<impl IntoResponse, HtmlError> {
    User::get_and_validate_text_content(&id, &user.id, &state.db).await?;

    TextContent::patch(&id, &form.context, &form.category, &form.text, &state.db).await?;

    if target.as_deref() == Some("latest_content_section") {
        let text_contents =
            truncate_text_contents(User::get_latest_text_contents(&user.id, &state.db).await?);

        return Ok(TemplateResponse::new_template(
            "dashboard/recent_content.html",
            RecentTextContentData { text_contents },
        ));
    }

    let (page_contents, pagination) = paginate_items(
        User::get_text_contents(&user.id, &state.db).await?,
        Some(1),
        CONTENTS_PER_PAGE,
    );
    let text_contents = truncate_text_contents(page_contents);
    let categories = User::get_user_categories(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_partial(
        "content/base.html",
        "main",
        ContentPageData {
            text_contents,
            categories,
            selected_category: None,
            pagination,
            page_query: String::new(),
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
    if let Some(file_info) = text_content.file_info.as_ref() {
        let file_in_use =
            TextContent::has_other_with_file(&file_info.id, &text_content.id, &state.db).await?;

        if !file_in_use {
            FileInfo::delete_by_id_with_storage(&file_info.id, &state.db, &state.storage).await?;
        }
    }

    // Delete related knowledge entities and text chunks
    KnowledgeEntity::delete_by_source_id(&id, &state.db).await?;
    TextChunk::delete_by_source_id(&id, &state.db).await?;

    // Delete the text content
    state.db.delete_item::<TextContent>(&id).await?;

    // Get updated content, categories and return the refreshed list
    let (page_contents, pagination) = paginate_items(
        User::get_text_contents(&user.id, &state.db).await?,
        Some(1),
        CONTENTS_PER_PAGE,
    );
    let text_contents = truncate_text_contents(page_contents);
    let categories = User::get_user_categories(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "content/content_list.html",
        ContentPageData {
            text_contents,
            categories,
            selected_category: None,
            pagination,
            page_query: String::new(),
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
        pub text_content: TextContent,
    }

    Ok(TemplateResponse::new_template(
        "content/read_content_modal.html",
        TextContentReadModalData { text_content },
    ))
}

pub async fn show_recent_content(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let text_contents =
        truncate_text_contents(User::get_latest_text_contents(&user.id, &state.db).await?);

    Ok(TemplateResponse::new_template(
        "dashboard/recent_content.html",
        RecentTextContentData { text_contents },
    ))
}
