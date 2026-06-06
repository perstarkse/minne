use axum::{
    extract::{Path, State},
    http::HeaderValue,
    Form,
};
use serde::{Deserialize, Serialize};

use common::{
    error::AppError,
    storage::types::{
        conversation::Conversation,
        message::{Message, MessageRole},
    },
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{
            template_as_response, template_with_headers, ResponseResult, TemplateResponse,
            TemplateResult,
        },
    },
};

#[derive(Serialize)]
pub struct ChatPageData {
    history: Vec<Message>,
    conversation: Option<Conversation>,
}

pub async fn show_chat_base(
    State(_state): State<HtmlState>,
    RequireUser(_user): RequireUser,
) -> TemplateResult {
    Ok(TemplateResponse::new_template(
        "chat/base.html",
        ChatPageData {
            history: vec![],
            conversation: None,
        },
    ))
}

#[derive(Deserialize)]
pub struct NewMessageForm {
    content: String,
}

pub async fn show_existing_chat(
    Path(conversation_id): Path<String>,
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> TemplateResult {
    let (conversation, messages) =
        Conversation::get_complete_conversation(conversation_id.as_str(), &user.id, &state.db)
            .await?;

    Ok(TemplateResponse::new_template(
        "chat/base.html",
        ChatPageData {
            history: messages,
            conversation: Some(conversation),
        },
    ))
}

pub async fn new_user_message(
    Path(conversation_id): Path<String>,
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<NewMessageForm>,
) -> ResponseResult {
    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
    }

    let conversation: Conversation = state
        .db
        .get_item(&conversation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("conversation was not found".into()))?;

    if conversation.user_id != user.id {
        return Ok(template_as_response(TemplateResponse::unauthorized()));
    }

    let user_message = Message::new(conversation_id, MessageRole::User, form.content, None);

    state.db.store_item(user_message.clone()).await?;

    let push_path = format!("/chat/{}", conversation.id);
    Ok(template_with_headers(
        TemplateResponse::new_template(
            "chat/streaming_response.html",
            SSEResponseInitData { user_message },
        ),
        |headers| {
            if let Ok(header_value) = HeaderValue::from_str(&push_path) {
                headers.insert("HX-Push", header_value);
            }
        },
    ))
}

pub async fn new_chat_user_message(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<NewMessageForm>,
) -> ResponseResult {
    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
        conversation: Conversation,
    }

    let conversation = Conversation::new(user.id.clone(), "New chat".to_string());
    let user_message = Message::new(
        conversation.id.clone(),
        MessageRole::User,
        form.content,
        None,
    );

    state.db.store_item(conversation.clone()).await?;
    state.db.store_item(user_message.clone()).await?;
    state.invalidate_conversation_archive_cache(&user.id).await;

    let push_path = format!("/chat/{}", conversation.id);
    Ok(template_with_headers(
        TemplateResponse::new_template(
            "chat/new_chat_first_response.html",
            SSEResponseInitData {
                user_message,
                conversation: conversation.clone(),
            },
        ),
        |headers| {
            if let Ok(header_value) = HeaderValue::from_str(&push_path) {
                headers.insert("HX-Push", header_value);
            }
        },
    ))
}

#[derive(Deserialize)]
pub struct PatchConversationTitle {
    title: String,
}

#[derive(Serialize)]
pub struct DrawerContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    edit_conversation_id: Option<String>,
}
pub async fn show_conversation_editing_title(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(conversation_id): Path<String>,
) -> TemplateResult {
    let conversation: Conversation = state
        .db
        .get_item(&conversation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("conversation not found".to_string()))?;

    if conversation.user_id != user.id {
        return Ok(TemplateResponse::unauthorized());
    }

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: Some(conversation_id),
        },
    ))
}

pub async fn patch_conversation_title(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(conversation_id): Path<String>,
    Form(form): Form<PatchConversationTitle>,
) -> TemplateResult {
    Conversation::patch_title(&conversation_id, &user.id, &form.title, &state.db).await?;
    state.invalidate_conversation_archive_cache(&user.id).await;

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: None,
        },
    ))
}

pub async fn delete_conversation(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(conversation_id): Path<String>,
) -> TemplateResult {
    let conversation: Conversation = state
        .db
        .get_item(&conversation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("conversation not found".to_string()))?;

    if conversation.user_id != user.id {
        return Ok(TemplateResponse::unauthorized());
    }

    state
        .db
        .delete_item::<Conversation>(&conversation_id)
        .await?;
    state.invalidate_conversation_archive_cache(&user.id).await;

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: None,
        },
    ))
}
pub async fn reload_sidebar(
    State(_state): State<HtmlState>,
    RequireUser(_user): RequireUser,
) -> TemplateResult {
    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: None,
        },
    ))
}
