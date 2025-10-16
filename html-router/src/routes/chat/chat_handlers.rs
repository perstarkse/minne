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
    user: User,
    history: Vec<Message>,
    conversation: Option<Conversation>,
    conversation_archive: Vec<Conversation>,
}

pub async fn show_initialized_chat(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<ChatStartParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation = Conversation::new(user.id.clone(), "Test".to_owned());

    let user_message = Message::new(
        conversation.id.to_string(),
        MessageRole::User,
        form.user_query,
        None,
    );

    let ai_message = Message::new(
        conversation.id.to_string(),
        MessageRole::AI,
        form.llm_response,
        Some(form.references),
    );

    state.db.store_item(conversation.clone()).await?;
    state.db.store_item(ai_message.clone()).await?;
    state.db.store_item(user_message.clone()).await?;

    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let messages = vec![user_message, ai_message];

    let mut response = TemplateResponse::new_template(
        "chat/base.html",
        ChatPageData {
            history: messages,
            user,
            conversation_archive,
            conversation: Some(conversation.clone()),
        },
    )
    .into_response();

    response.headers_mut().insert(
        "HX-Push",
        HeaderValue::from_str(&format!("/chat/{}", conversation.id)).unwrap(),
    );

    Ok(response)
}

pub async fn show_chat_base(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "chat/base.html",
        ChatPageData {
            history: vec![],
            user,
            conversation_archive,
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
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let (conversation, messages) =
        Conversation::get_complete_conversation(conversation_id.as_str(), &user.id, &state.db)
            .await?;

    Ok(TemplateResponse::new_template(
        "chat/base.html",
        ChatPageData {
            history: messages,
            user,
            conversation: Some(conversation),
            conversation_archive,
        },
    ))
}

pub async fn new_user_message(
    Path(conversation_id): Path<String>,
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<NewMessageForm>,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation: Conversation = state
        .db
        .get_item(&conversation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Conversation was not found".into()))?;

    if conversation.user_id != user.id {
        return Ok(TemplateResponse::unauthorized().into_response());
    }

    let user_message = Message::new(conversation_id, MessageRole::User, form.content, None);

    state.db.store_item(user_message.clone()).await?;

    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
    }

    let mut response = TemplateResponse::new_template(
        "chat/streaming_response.html",
        SSEResponseInitData { user_message },
    )
    .into_response();

    response.headers_mut().insert(
        "HX-Push",
        HeaderValue::from_str(&format!("/chat/{}", conversation.id)).unwrap(),
    );

    Ok(response)
}

pub async fn new_chat_user_message(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<NewMessageForm>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let conversation = Conversation::new(user.id, "New chat".to_string());
    let user_message = Message::new(
        conversation.id.clone(),
        MessageRole::User,
        form.content,
        None,
    );

    state.db.store_item(conversation.clone()).await?;
    state.db.store_item(user_message.clone()).await?;

    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
        conversation: Conversation,
    }
    let mut response = TemplateResponse::new_template(
        "chat/new_chat_first_response.html",
        SSEResponseInitData {
            user_message,
            conversation: conversation.clone(),
        },
    )
    .into_response();

    response.headers_mut().insert(
        "HX-Push",
        HeaderValue::from_str(&format!("/chat/{}", conversation.id)).unwrap(),
    );

    Ok(response.into_response())
}

#[derive(Deserialize)]
pub struct PatchConversationTitle {
    title: String,
}

#[derive(Serialize)]
pub struct DrawerContext {
    user: User,
    conversation_archive: Vec<Conversation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edit_conversation_id: Option<String>,
}
pub async fn show_conversation_editing_title(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(conversation_id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let owns = conversation_archive
        .iter()
        .any(|c| c.id == conversation_id && c.user_id == user.id);
    if !owns {
        return Ok(TemplateResponse::unauthorized().into_response());
    }

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            user,
            conversation_archive,
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

    let updated_conversations = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            user,
            conversation_archive: updated_conversations,
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
        .ok_or_else(|| AppError::NotFound("Conversation not found".to_string()))?;

    if conversation.user_id != user.id {
        return Ok(TemplateResponse::unauthorized().into_response());
    }

    state
        .db
        .delete_item::<Conversation>(&conversation_id)
        .await?;

    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            user,
            conversation_archive,
            edit_conversation_id: None,
        },
    )
    .into_response())
}
pub async fn reload_sidebar(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "sidebar.html",
        DrawerContext {
            user,
            conversation_archive,
            edit_conversation_id: None,
        },
    )
    .into_response())
}
