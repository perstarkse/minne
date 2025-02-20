use axum::{
    extract::State,
    response::{IntoResponse, Redirect},
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::HtmlError,
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::user::User,
};

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
});

#[derive(Deserialize, Debug, Serialize)]
pub enum MessageRole {
    User,
    AI,
    System,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct Message {
    conversation_id: String,
    role: MessageRole,
    content: String,
    references: Option<Vec<String>>,
}

pub struct Conversation {
    user_id: String,
    title: String,
}

pub async fn show_initialized_chat(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<ChatStartParams>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying chat start");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    info!("{:?}", form);

    let user_message = Message {
        conversation_id: "test".to_string(),
        role: MessageRole::User,
        content: form.user_query,
        references: None,
    };

    let ai_message = Message {
        conversation_id: "test".to_string(),
        role: MessageRole::AI,
        content: form.llm_response,
        references: Some(form.references),
    };

    let messages = vec![user_message, ai_message];

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: messages,
            user,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn show_chat_base(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying empty chat start");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: vec![],
            user,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
