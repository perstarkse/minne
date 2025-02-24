use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Redirect, Sse,
    },
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use futures::{stream, Stream, StreamExt};
use surrealdb::{engine::any::Any, Surreal};
use tokio::time::sleep;
use tracing::info;
use uuid::Uuid;

use crate::{
    error::HtmlError,
    page_data,
    server::{routes::html::render_template, AppState},
    storage::types::{
        message::{Message, MessageRole},
        user::User,
    },
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
    conversation_id: String,
});

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

    let conversation_id = Uuid::new_v4().to_string();

    let user_message = Message::new("test".to_string(), MessageRole::User, form.user_query, None);

    let ai_message = Message::new(
        "test".to_string(),
        MessageRole::AI,
        form.llm_response,
        Some(form.references),
    );

    let messages = vec![user_message, ai_message];

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: messages,
            user,
            conversation_id,
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

    let conversation_id = Uuid::new_v4().to_string();

    let output = render_template(
        ChatData::template_name(),
        ChatData {
            history: vec![],
            user,
            conversation_id,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

#[derive(Deserialize)]
pub struct NewMessageForm {
    content: String,
}

pub async fn new_user_message(
    Path(conversation_id): Path<String>,
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<NewMessageForm>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying empty chat start");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let query_id = Uuid::new_v4().to_string();
    let user_message = form.content.clone();

    // Save to database
    // state
    //     .db
    //     .save(conversation_id, query_id.clone(), user_message)
    //     .await;

    #[derive(Serialize)]
    struct SSEResponseInitData {
        user_message: String,
        query_id: String,
    }

    let output = render_template(
        "chat/streaming_response.html",
        SSEResponseInitData {
            user_message,
            query_id,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

#[derive(Deserialize)]
pub struct QueryParams {
    query_id: String,
}

pub async fn get_response_stream(
    State(_state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Query(params): Query<QueryParams>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let stream = stream::iter(vec![
        Event::default()
            .event("chat_message")
            .data("Hello, starting stream!"),
        Event::default()
            .event("chat_message")
            .data("This is message 2"),
        Event::default().event("chat_message").data("Final message"),
        Event::default()
            .event("close_stream")
            .data("Stream complete"), // Signal to close
    ])
    .then(|event| async move {
        sleep(Duration::from_millis(500)).await; // Delay between messages
        Ok(event)
    });

    info!("Streaming started");

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
