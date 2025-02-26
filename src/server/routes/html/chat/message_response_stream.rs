use std::{pin::Pin, time::Duration};

use axum::{
    extract::{Query, State},
    response::{sse::Event, Sse},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use futures::{stream, Stream, StreamExt, TryStreamExt};
use json_stream_parser::JsonStreamParser;
use serde::Deserialize;
use surrealdb::{engine::any::Any, Surreal};
use tracing::{error, info};

use crate::{
    retrieval::{
        combined_knowledge_entity_retrieval,
        query_helper::{
            create_chat_request, create_user_message, format_entities_json, LLMResponseFormat,
        },
    },
    server::AppState,
    storage::{
        db::{get_item, store_item, SurrealDbClient},
        types::{
            message::{Message, MessageRole},
            user::User,
        },
    },
};

fn create_error_stream(
    message: impl Into<String>,
) -> Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> {
    let message = message.into();
    stream::once(async move { Ok(Event::default().event("error").data(message)) }).boxed()
}

// Helper function to get message and user
async fn get_message_and_user(
    db: &SurrealDbClient,
    current_user: Option<User>,
    message_id: &str,
) -> Result<(Message, User), Sse<Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>>>> {
    // Check authentication
    let user = match current_user {
        Some(user) => user,
        None => {
            return Err(Sse::new(create_error_stream(
                "You must be signed in to use this feature",
            )))
        }
    };

    // Retrieve message
    let message = match get_item::<Message>(db, message_id).await {
        Ok(Some(message)) => message,
        Ok(None) => {
            return Err(Sse::new(create_error_stream(
                "Message not found: the specified message does not exist",
            )))
        }
        Err(e) => {
            error!("Database error retrieving message {}: {:?}", message_id, e);
            return Err(Sse::new(create_error_stream(
                "Failed to retrieve message: database error",
            )));
        }
    };

    Ok((message, user))
}

#[derive(Deserialize)]
pub struct QueryParams {
    message_id: String,
}

pub async fn get_response_stream(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Query(params): Query<QueryParams>,
) -> Sse<Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>>> {
    // 1. Authentication and initial data validation
    let (user_message, user) = match get_message_and_user(
        &state.surreal_db_client,
        auth.current_user,
        &params.message_id,
    )
    .await
    {
        Ok((user_message, user)) => (user_message, user),
        Err(error_stream) => return error_stream,
    };

    // 2. Retrieve knowledge entities
    let entities = match combined_knowledge_entity_retrieval(
        &state.surreal_db_client,
        &state.openai_client,
        &user_message.content,
        &user.id,
    )
    .await
    {
        Ok(entities) => entities,
        Err(_e) => {
            return Sse::new(create_error_stream("Failed to retrieve knowledge entities"));
        }
    };

    // 3. Create the OpenAI request
    let entities_json = format_entities_json(&entities);
    let formatted_user_message = create_user_message(&entities_json, &user_message.content);
    let request = match create_chat_request(formatted_user_message) {
        Ok(req) => req,
        Err(..) => {
            return Sse::new(create_error_stream("Failed to create chat request"));
        }
    };

    // 4. Set up the OpenAI stream
    let openai_stream = match state.openai_client.chat().create_stream(request).await {
        Ok(stream) => stream,
        Err(_e) => {
            return Sse::new(create_error_stream("Failed to create OpenAI stream"));
        }
    };

    // 5. Create channel for collecting complete response
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1000);
    let tx_clone = tx.clone();

    // 6. Set up the collection task for DB storage
    let db_client = state.surreal_db_client.clone();
    tokio::spawn(async move {
        drop(tx); // Close sender when no longer needed

        // Collect full response
        let mut full_json = String::new();
        while let Some(chunk) = rx.recv().await {
            full_json.push_str(&chunk);
        }

        // Try to extract structured data
        if let Ok(response) = serde_json::from_str::<LLMResponseFormat>(&full_json) {
            let references: Vec<String> = response
                .references
                .into_iter()
                .map(|r| r.reference)
                .collect();

            let ai_message = Message::new(
                user_message.conversation_id,
                MessageRole::AI,
                response.answer,
                Some(references),
            );

            match store_item(&db_client, ai_message).await {
                Ok(_) => info!("Successfully stored AI message with references"),
                Err(e) => error!("Failed to store AI message: {:?}", e),
            }
        } else {
            error!("Failed to parse LLM response as structured format");

            // Fallback - store raw response
            let ai_message = Message::new(
                user_message.conversation_id,
                MessageRole::AI,
                full_json,
                Some(vec![]),
            );

            let _ = store_item(&db_client, ai_message).await;
        }
    });

    // Create a shared state for tracking the JSON parsing
    let json_state = std::sync::Arc::new(tokio::sync::Mutex::new(StreamParserState::new()));

    // 7. Create the response event stream
    let event_stream = openai_stream
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        .map(move |result| {
            let tx_storage = tx_clone.clone();
            let json_state = json_state.clone();

            async move {
                match result {
                    Ok(response) => {
                        let content = response
                            .choices
                            .first()
                            .and_then(|choice| choice.delta.content.clone())
                            .unwrap_or_default();

                        if !content.is_empty() {
                            // Always send raw content to storage
                            let _ = tx_storage.send(content.clone()).await;

                            // Process through JSON parser
                            let mut state = json_state.lock().await;

                            let display_content = state.process_chunk(&content);
                            drop(state);
                            if !display_content.is_empty() {
                                return Ok(Event::default()
                                    .event("chat_message")
                                    .data(display_content));
                            }

                            // Empty or filtered content
                            Ok(Event::default().event("chat_message").data(""))
                        } else {
                            Ok(Event::default().event("chat_message").data(""))
                        }
                    }
                    Err(e) => Ok(Event::default()
                        .event("error")
                        .data(format!("Stream error: {}", e))),
                }
            }
        })
        .buffered(10)
        .chain(stream::once(async {
            Ok(Event::default()
                .event("close_stream")
                .data("Stream complete"))
        }));

    info!("OpenAI streaming started");
    Sse::new(event_stream.boxed()).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

// Replace JsonParseState with StreamParserState
struct StreamParserState {
    parser: JsonStreamParser,
    last_answer_content: String,
    in_answer_field: bool,
}

impl StreamParserState {
    fn new() -> Self {
        Self {
            parser: JsonStreamParser::new(),
            last_answer_content: String::new(),
            in_answer_field: false,
        }
    }

    fn process_chunk(&mut self, chunk: &str) -> String {
        // Feed all characters into the parser
        for c in chunk.chars() {
            let _ = self.parser.add_char(c);
        }

        // Get the current state of the JSON
        // The get_result() method returns a &Value, not a Result
        let json = self.parser.get_result();

        // Check if we're in the answer field
        if let Some(obj) = json.as_object() {
            if let Some(answer) = obj.get("answer") {
                self.in_answer_field = true;

                // Get current answer content
                let current_content = answer.as_str().unwrap_or_default().to_string();

                // Calculate difference to send only new content
                if current_content.len() > self.last_answer_content.len() {
                    let new_content = current_content[self.last_answer_content.len()..].to_string();
                    self.last_answer_content = current_content;
                    return new_content;
                }
            }
        }

        // No new content to return
        String::new()
    }
}

// 7. Create the response event stream
//     let event_stream = openai_stream
//         .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
//         .map(move |result| {
//             let tx = tx_clone.clone();
//             async move {
//                 match result {
//                     Ok(response) => {
//                         let content = response
//                             .choices
//                             .first()
//                             .and_then(|choice| choice.delta.content.clone())
//                             .unwrap_or_default();

//                         if !content.is_empty() {
//                             let _ = tx.send(content.clone()).await;
//                             Ok(Event::default().event("chat_message").data(content))
//                         } else {
//                             Ok(Event::default().event("chat_message").data(""))
//                         }
//                     }
//                     Err(e) => Ok(Event::default()
//                         .event("error")
//                         .data(format!("Stream error: {}", e))),
//                 }
//             }
//         })
//         .buffered(10)
//         .chain(stream::once(async {
//             Ok(Event::default()
//                 .event("close_stream")
//                 .data("Stream complete"))
//         }));

//     info!("OpenAI streaming started");

//     Sse::new(event_stream.boxed()).keep_alive(
//         axum::response::sse::KeepAlive::new()
//             .interval(Duration::from_secs(15))
//             .text("keep-alive"),
//     )
// }
