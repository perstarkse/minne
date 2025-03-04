pub mod message_response_stream;
pub mod references;

use axum::{
    extract::{Path, State},
    http::HeaderValue,
    response::{IntoResponse, Redirect},
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use common::{
    error::{AppError, HtmlError},
    storage::{
        db::{get_item, store_item},
        types::{
            conversation::Conversation,
            message::{Message, MessageRole},
            user::User,
        },
    },
};

use crate::{html_state::HtmlState, page_data, routes::render_template};

// Update your ChatStartParams struct to properly deserialize the references
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

page_data!(ChatData, "chat/base.html", {
    user: User,
    history: Vec<Message>,
    conversation: Option<Conversation>,
    conversation_archive: Vec<Conversation>
});

pub async fn show_initialized_chat(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<ChatStartParams>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying chat start");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

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

    let (conversation_result, ai_message_result, user_message_result) = futures::join!(
        store_item(&state.surreal_db_client, conversation.clone()),
        store_item(&state.surreal_db_client, ai_message.clone()),
        store_item(&state.surreal_db_client, user_message.clone())
    );

    // Check each result individually
    conversation_result.map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;
    user_message_result.map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;
    ai_message_result.map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    let conversation_archive = User::get_user_conversations(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let messages = vec![user_message, ai_message];

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: messages,
            user,
            conversation_archive,
            conversation: Some(conversation.clone()),
        },
        state.templates.clone(),
    )?;

    let mut response = output.into_response();
    response.headers_mut().insert(
        "HX-Push",
        HeaderValue::from_str(&format!("/chat/{}", conversation.id)).unwrap(),
    );
    Ok(response)
}

pub async fn show_chat_base(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying empty chat start");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let conversation_archive = User::get_user_conversations(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: vec![],
            user,
            conversation_archive,
            conversation: None,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

#[derive(Deserialize)]
pub struct NewMessageForm {
    content: String,
}

pub async fn show_existing_chat(
    Path(conversation_id): Path<String>,
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying initialized chat with id: {}", conversation_id);

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let conversation_archive = User::get_user_conversations(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let (conversation, messages) = Conversation::get_complete_conversation(
        conversation_id.as_str(),
        &user.id,
        &state.surreal_db_client,
    )
    .await
    .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: messages,
            user,
            conversation: Some(conversation.clone()),
            conversation_archive,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn new_user_message(
    Path(conversation_id): Path<String>,
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<NewMessageForm>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let conversation: Conversation = get_item(&state.surreal_db_client, &conversation_id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?
        .ok_or_else(|| {
            HtmlError::new(
                AppError::NotFound("Conversation was not found".to_string()),
                state.templates.clone(),
            )
        })?;

    if conversation.user_id != user.id {
        return Err(HtmlError::new(
            AppError::Auth("The user does not have permission for this conversation".to_string()),
            state.templates.clone(),
        ));
    };

    let user_message = Message::new(conversation_id, MessageRole::User, form.content, None);

    store_item(&state.surreal_db_client, user_message.clone())
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
    }

    let output = render_template(
        "chat/streaming_response.html",
        SSEResponseInitData { user_message },
        state.templates.clone(),
    )?;

    let mut response = output.into_response();
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

    store_item(&state.surreal_db_client, conversation.clone())
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;
    store_item(&state.surreal_db_client, user_message.clone())
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: Message,
        conversation: Conversation,
    }

    let output = render_template(
        "chat/new_chat_first_response.html",
        SSEResponseInitData {
            user_message,
            conversation: conversation.clone(),
        },
        state.templates.clone(),
    )?;

    let mut response = output.into_response();
    response.headers_mut().insert(
        "HX-Push",
        HeaderValue::from_str(&format!("/chat/{}", conversation.id)).unwrap(),
    );
    Ok(response)
}
