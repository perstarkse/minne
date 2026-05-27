use axum::{
    extract::{Path, State},
    http::HeaderValue,
    response::{IntoResponse, Redirect},
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};

use common::{
    error::AppError,
    storage::types::{
        conversation::Conversation,
        message::{Message, MessageRole},
        user::User,
    },
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

#[derive(Debug, Deserialize)]
pub struct ChatStartParams {
    user_query: String,
    llm_response: String,
    #[serde(deserialize_with = "deserialize_references")]
    references: Vec<String>,
}

// Custom deserializer function
fn deserialize_references<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    serde_json::from_str(&s).map_err(serde::de::Error::custom)
}

#[derive(Serialize)]
pub struct ChatPageData {
    history: Vec<Message>,
    conversation: Option<Conversation>,
}

/// # Panics
/// Panics if the HX-Push header value cannot be parsed.
pub async fn show_initialized_chat(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<ChatStartParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation = Conversation::new(user.id.clone(), "Test".to_owned());

    let user_message = Message::new(
        conversation.id.clone(),
        MessageRole::User,
        form.user_query,
        None,
    );

    let ai_message = Message::new(
        conversation.id.clone(),
        MessageRole::AI,
        form.llm_response,
        Some(form.references),
    );

    state.db.store_item(conversation.clone()).await?;
    state.db.store_item(ai_message.clone()).await?;
    state.db.store_item(user_message.clone()).await?;
    state.invalidate_conversation_archive_cache(&user.id).await;

    let messages = vec![user_message, ai_message];

    let mut response = TemplateResponse::new_template(
        "chat/base.html",
        ChatPageData {
            history: messages,
            conversation: Some(conversation.clone()),
        },
    )
    .into_response();

    if let Ok(header_value) = HeaderValue::from_str(&format!("/chat/{}", conversation.id)) {
        response.headers_mut().insert("HX-Push", header_value);
    }

    Ok(response)
}

pub async fn show_chat_base(
    State(_state): State<HtmlState>,
    RequireUser(_user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
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
) -> Result<impl IntoResponse, HtmlError> {
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

/// # Panics
/// Panics if the HX-Push header value cannot be parsed.
pub async fn new_user_message(
    Path(conversation_id): Path<String>,
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<NewMessageForm>,
) -> Result<impl IntoResponse, HtmlError> {
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
        return Ok(TemplateResponse::unauthorized().into_response());
    }

    let user_message = Message::new(conversation_id, MessageRole::User, form.content, None);

    state.db.store_item(user_message.clone()).await?;

    let mut response = TemplateResponse::new_template(
        "chat/streaming_response.html",
        SSEResponseInitData { user_message },
    )
    .into_response();

    if let Ok(header_value) = HeaderValue::from_str(&format!("/chat/{}", conversation.id)) {
        response.headers_mut().insert("HX-Push", header_value);
    }

    Ok(response)
}

/// # Panics
/// Panics if the HX-Push header value cannot be parsed.
pub async fn new_chat_user_message(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<NewMessageForm>,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
        conversation: Conversation,
    }

    let Some(user) = auth.current_user else {
        return Ok(Redirect::to("/").into_response());
    };

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

    let mut response = TemplateResponse::new_template(
        "chat/new_chat_first_response.html",
        SSEResponseInitData {
            user_message,
            conversation: conversation.clone(),
        },
    )
    .into_response();

    if let Ok(header_value) = HeaderValue::from_str(&format!("/chat/{}", conversation.id)) {
        response.headers_mut().insert("HX-Push", header_value);
    }

    Ok(response.into_response())
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
) -> Result<impl IntoResponse, HtmlError> {
    let conversation: Conversation = state
        .db
        .get_item(&conversation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("conversation not found".to_string()))?;

    if conversation.user_id != user.id {
        return Ok(TemplateResponse::unauthorized().into_response());
    }

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: Some(conversation_id),
        },
    )
    .into_response())
}

pub async fn patch_conversation_title(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(conversation_id): Path<String>,
    Form(form): Form<PatchConversationTitle>,
) -> Result<impl IntoResponse, HtmlError> {
    Conversation::patch_title(&conversation_id, &user.id, &form.title, &state.db).await?;
    state.invalidate_conversation_archive_cache(&user.id).await;

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: None,
        },
    )
    .into_response())
}

pub async fn delete_conversation(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(conversation_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation: Conversation = state
        .db
        .get_item(&conversation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("conversation not found".to_string()))?;

    if conversation.user_id != user.id {
        return Ok(TemplateResponse::unauthorized().into_response());
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
    )
    .into_response())
}
pub async fn reload_sidebar(
    State(_state): State<HtmlState>,
    RequireUser(_user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            edit_conversation_id: None,
        },
    )
    .into_response())
}
