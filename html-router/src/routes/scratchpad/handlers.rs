use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Form,
};
use axum_htmx::{HxBoosted, HxRequest, HX_TRIGGER};
use serde::{Deserialize, Serialize};

use crate::html_state::HtmlState;
use crate::middlewares::{
    auth_middleware::RequireUser,
    response_middleware::{HtmlError, TemplateResponse},
};
use common::storage::types::{
    conversation::Conversation, ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
    scratchpad::Scratchpad, user::User,
};

#[derive(Serialize)]
pub struct ScratchpadPageData {
    user: User,
    scratchpads: Vec<ScratchpadListItem>,
    archived_scratchpads: Vec<ScratchpadArchiveItem>,
    conversation_archive: Vec<Conversation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_scratchpad: Option<ScratchpadDetail>,
}

#[derive(Serialize)]
pub struct ScratchpadListItem {
    id: String,
    title: String,
    content: String,
    last_saved_at: String,
}

#[derive(Serialize)]
pub struct ScratchpadDetailData {
    user: User,
    scratchpad: ScratchpadDetail,
    conversation_archive: Vec<Conversation>,
}

#[derive(Serialize)]
pub struct ScratchpadArchiveItem {
    id: String,
    title: String,
    archived_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ingested_at: Option<String>,
}

#[derive(Serialize)]
pub struct ScratchpadDetail {
    id: String,
    title: String,
    content: String,
    created_at: String,
    updated_at: String,
    last_saved_at: String,
    is_dirty: bool,
}

#[derive(Serialize)]
pub struct AutoSaveResponse {
    success: bool,
    last_saved_at_display: String,
    last_saved_at_iso: String,
}

impl From<&Scratchpad> for ScratchpadListItem {
    fn from(value: &Scratchpad) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            content: value.content.clone(),
            last_saved_at: value.last_saved_at.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}

impl From<&Scratchpad> for ScratchpadArchiveItem {
    fn from(value: &Scratchpad) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            archived_at: value
                .archived_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "Unknown".to_string()),
            ingested_at: value
                .ingested_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string()),
        }
    }
}

impl From<&Scratchpad> for ScratchpadDetail {
    fn from(value: &Scratchpad) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            content: value.content.clone(),
            created_at: value.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            updated_at: value.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            last_saved_at: value.last_saved_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            is_dirty: value.is_dirty,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateScratchpadForm {
    title: String,
}

#[derive(Deserialize)]
pub struct UpdateScratchpadForm {
    content: String,
}

#[derive(Deserialize)]
pub struct UpdateTitleForm {
    title: String,
}

#[derive(Deserialize)]
pub struct EditTitleQuery {
    edit_title: Option<bool>,
}

pub async fn show_scratchpad_page(
    RequireUser(user): RequireUser,
    HxRequest(is_htmx): HxRequest,
    HxBoosted(is_boosted): HxBoosted,
    State(state): State<HtmlState>,
) -> Result<impl IntoResponse, HtmlError> {
    let scratchpads = Scratchpad::get_by_user(&user.id, &state.db).await?;
    let archived_scratchpads = Scratchpad::get_archived_by_user(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let scratchpad_list: Vec<ScratchpadListItem> =
        scratchpads.iter().map(ScratchpadListItem::from).collect();
    let archived_list: Vec<ScratchpadArchiveItem> = archived_scratchpads
        .iter()
        .map(ScratchpadArchiveItem::from)
        .collect();

    if is_htmx && !is_boosted {
        Ok(TemplateResponse::new_partial(
            "scratchpad/base.html",
            "main",
            ScratchpadPageData {
                user,
                scratchpads: scratchpad_list,
                archived_scratchpads: archived_list,
                conversation_archive,
                new_scratchpad: None,
            },
        ))
    } else {
        Ok(TemplateResponse::new_template(
            "scratchpad/base.html",
            ScratchpadPageData {
                user,
                scratchpads: scratchpad_list,
                archived_scratchpads: archived_list,
                conversation_archive,
                new_scratchpad: None,
            },
        ))
    }
}

pub async fn show_scratchpad_modal(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
    Query(query): Query<EditTitleQuery>,
) -> Result<impl IntoResponse, HtmlError> {
    let scratchpad = Scratchpad::get_by_id(&scratchpad_id, &user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let scratchpad_detail = ScratchpadDetail::from(&scratchpad);

    // Handle edit_title query parameter if needed in future
    let _ = query.edit_title.unwrap_or(false);

    Ok(TemplateResponse::new_template(
        "scratchpad/editor_modal.html",
        ScratchpadDetailData {
            user,
            scratchpad: scratchpad_detail,
            conversation_archive,
        },
    ))
}

pub async fn create_scratchpad(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Form(form): Form<CreateScratchpadForm>,
) -> Result<impl IntoResponse, HtmlError> {
    let user_id = user.id.clone();
    let scratchpad = Scratchpad::new(user_id.clone(), form.title);
    let _stored = state.db.store_item(scratchpad.clone()).await?;

    let scratchpads = Scratchpad::get_by_user(&user.id, &state.db).await?;
    let archived_scratchpads = Scratchpad::get_archived_by_user(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let scratchpad_list: Vec<ScratchpadListItem> =
        scratchpads.iter().map(ScratchpadListItem::from).collect();
    let archived_list: Vec<ScratchpadArchiveItem> = archived_scratchpads
        .iter()
        .map(ScratchpadArchiveItem::from)
        .collect();

    Ok(TemplateResponse::new_partial(
        "scratchpad/base.html",
        "main",
        ScratchpadPageData {
            user,
            scratchpads: scratchpad_list,
            archived_scratchpads: archived_list,
            conversation_archive,
            new_scratchpad: Some(ScratchpadDetail::from(&scratchpad)),
        },
    ))
}

pub async fn auto_save_scratchpad(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
    Form(form): Form<UpdateScratchpadForm>,
) -> Result<impl IntoResponse, HtmlError> {
    let updated =
        Scratchpad::update_content(&scratchpad_id, &user.id, &form.content, &state.db).await?;

    // Return a success indicator for auto-save
    Ok(axum::Json(AutoSaveResponse {
        success: true,
        last_saved_at_display: updated
            .last_saved_at
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        last_saved_at_iso: updated.last_saved_at.to_rfc3339(),
    }))
}

pub async fn update_scratchpad_title(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
    Form(form): Form<UpdateTitleForm>,
) -> Result<impl IntoResponse, HtmlError> {
    Scratchpad::update_title(&scratchpad_id, &user.id, &form.title, &state.db).await?;

    let scratchpad = Scratchpad::get_by_id(&scratchpad_id, &user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "scratchpad/editor_modal.html",
        ScratchpadDetailData {
            user,
            scratchpad: ScratchpadDetail::from(&scratchpad),
            conversation_archive,
        },
    ))
}

pub async fn delete_scratchpad(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    Scratchpad::delete(&scratchpad_id, &user.id, &state.db).await?;

    // Return the updated main section content
    let scratchpads = Scratchpad::get_by_user(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;
    let archived_scratchpads = Scratchpad::get_archived_by_user(&user.id, &state.db).await?;

    let scratchpad_list: Vec<ScratchpadListItem> =
        scratchpads.iter().map(ScratchpadListItem::from).collect();
    let archived_list: Vec<ScratchpadArchiveItem> = archived_scratchpads
        .iter()
        .map(ScratchpadArchiveItem::from)
        .collect();

    Ok(TemplateResponse::new_partial(
        "scratchpad/base.html",
        "main",
        ScratchpadPageData {
            user,
            scratchpads: scratchpad_list,
            archived_scratchpads: archived_list,
            conversation_archive,
            new_scratchpad: None,
        },
    ))
}

pub async fn ingest_scratchpad(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let scratchpad = Scratchpad::get_by_id(&scratchpad_id, &user.id, &state.db).await?;

    if scratchpad.content.trim().is_empty() {
        let trigger_payload = serde_json::json!({
            "toast": {
                "title": "Ingestion skipped",
                "description": "Cannot ingest an empty scratchpad.",
                "type": "warning"
            }
        });
        let trigger_value = serde_json::to_string(&trigger_payload).unwrap_or_else(|_| {
            r#"{"toast":{"title":"Ingestion skipped","description":"Cannot ingest an empty scratchpad.","type":"warning"}}"#.to_string()
        });

        let mut response = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::empty())
            .unwrap_or_else(|_| Response::new(axum::body::Body::empty()));

        if let Ok(header_value) = HeaderValue::from_str(&trigger_value) {
            response.headers_mut().insert(HX_TRIGGER, header_value);
        }

        return Ok(response);
    }

    // Create ingestion task

    let payload = IngestionPayload::Text {
        text: scratchpad.content.clone(),
        context: format!("Scratchpad: {}", scratchpad.title),
        category: "scratchpad".to_string(),
        user_id: user.id.clone(),
    };

    let task = IngestionTask::new(payload, user.id.clone());
    state.db.store_item(task).await?;

    // Archive the scratchpad once queued for ingestion
    Scratchpad::archive(&scratchpad_id, &user.id, &state.db, true).await?;

    let scratchpads = Scratchpad::get_by_user(&user.id, &state.db).await?;
    let archived_scratchpads = Scratchpad::get_archived_by_user(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let scratchpad_list: Vec<ScratchpadListItem> =
        scratchpads.iter().map(ScratchpadListItem::from).collect();
    let archived_list: Vec<ScratchpadArchiveItem> = archived_scratchpads
        .iter()
        .map(ScratchpadArchiveItem::from)
        .collect();

    let trigger_payload = serde_json::json!({
        "toast": {
            "title": "Ingestion queued",
            "description": format!("\"{}\" archived and added to the ingestion queue.", scratchpad.title),
            "type": "success"
        }
    });
    let trigger_value = serde_json::to_string(&trigger_payload).unwrap_or_else(|_| {
        r#"{"toast":{"title":"Ingestion queued","description":"Scratchpad archived and added to the ingestion queue.","type":"success"}}"#.to_string()
    });

    let template_response = TemplateResponse::new_partial(
        "scratchpad/base.html",
        "main",
        ScratchpadPageData {
            user,
            scratchpads: scratchpad_list,
            archived_scratchpads: archived_list,
            conversation_archive,
            new_scratchpad: None,
        },
    );

    let mut response = template_response.into_response();
    if let Ok(header_value) = HeaderValue::from_str(&trigger_value) {
        response.headers_mut().insert(HX_TRIGGER, header_value);
    }

    Ok(response)
}

pub async fn restore_scratchpad(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    Scratchpad::restore(&scratchpad_id, &user.id, &state.db).await?;

    let scratchpads = Scratchpad::get_by_user(&user.id, &state.db).await?;
    let archived_scratchpads = Scratchpad::get_archived_by_user(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let scratchpad_list: Vec<ScratchpadListItem> =
        scratchpads.iter().map(ScratchpadListItem::from).collect();
    let archived_list: Vec<ScratchpadArchiveItem> = archived_scratchpads
        .iter()
        .map(ScratchpadArchiveItem::from)
        .collect();

    let trigger_payload = serde_json::json!({
        "toast": {
            "title": "Scratchpad restored",
            "description": "The scratchpad is back in your active list.",
            "type": "info"
        }
    });
    let trigger_value = serde_json::to_string(&trigger_payload).unwrap_or_else(|_| {
        r#"{"toast":{"title":"Scratchpad restored","description":"The scratchpad is back in your active list.","type":"info"}}"#.to_string()
    });

    let template_response = TemplateResponse::new_partial(
        "scratchpad/base.html",
        "main",
        ScratchpadPageData {
            user,
            scratchpads: scratchpad_list,
            archived_scratchpads: archived_list,
            conversation_archive,
            new_scratchpad: None,
        },
    );

    let mut response = template_response.into_response();
    if let Ok(header_value) = HeaderValue::from_str(&trigger_value) {
        response.headers_mut().insert(HX_TRIGGER, header_value);
    }

    Ok(response)
}

pub async fn archive_scratchpad(
    RequireUser(user): RequireUser,
    State(state): State<HtmlState>,
    Path(scratchpad_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    Scratchpad::archive(&scratchpad_id, &user.id, &state.db, false).await?;

    let scratchpads = Scratchpad::get_by_user(&user.id, &state.db).await?;
    let archived_scratchpads = Scratchpad::get_archived_by_user(&user.id, &state.db).await?;
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let scratchpad_list: Vec<ScratchpadListItem> =
        scratchpads.iter().map(ScratchpadListItem::from).collect();
    let archived_list: Vec<ScratchpadArchiveItem> = archived_scratchpads
        .iter()
        .map(ScratchpadArchiveItem::from)
        .collect();

    let trigger_payload = serde_json::json!({
        "toast": {
            "title": "Scratchpad archived",
            "description": "You can find it in the archive drawer below.",
            "type": "warning"
        }
    });
    let trigger_value = serde_json::to_string(&trigger_payload).unwrap_or_else(|_| {
        r#"{"toast":{"title":"Scratchpad archived","description":"You can find it in the archive drawer below.","type":"warning"}}"#.to_string()
    });

    let template_response = TemplateResponse::new_partial(
        "scratchpad/base.html",
        "main",
        ScratchpadPageData {
            user,
            scratchpads: scratchpad_list,
            archived_scratchpads: archived_list,
            conversation_archive,
            new_scratchpad: None,
        },
    );

    let mut response = template_response.into_response();
    if let Ok(header_value) = HeaderValue::from_str(&trigger_value) {
        response.headers_mut().insert(HX_TRIGGER, header_value);
    }

    Ok(response)
}
