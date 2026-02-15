#![allow(clippy::missing_docs_in_private_items)]

use std::{pin::Pin, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, KeepAlive, KeepAliveStream},
        Sse,
    },
};
use futures::{
    stream::{self, once},
    Stream, StreamExt, TryStreamExt,
};
use json_stream_parser::JsonStreamParser;
use minijinja::Value;
use retrieval_pipeline::{
    answer_retrieval::{
        chunks_to_chat_context, create_chat_request, create_user_message_with_history,
        LLMResponseFormat,
    },
    retrieved_entities_to_json,
};
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use tokio::sync::{mpsc::channel, Mutex};
use tracing::{debug, error, info};

use common::storage::{
    db::SurrealDbClient,
    types::{
        conversation::Conversation,
        message::{Message, MessageRole},
        system_settings::SystemSettings,
        user::User,
    },
};

use crate::{html_state::HtmlState, AuthSessionType};

use super::reference_validation::{collect_reference_ids_from_retrieval, validate_references};

type EventStream = Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>>;
type SseResponse = Sse<KeepAliveStream<EventStream>>;

fn sse_with_keep_alive(stream: EventStream) -> SseResponse {
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

// Error handling function
fn create_error_stream(message: impl Into<String>) -> EventStream {
    let message = message.into();
    stream::once(async move { Ok(Event::default().event("error").data(message)) }).boxed()
}

// Helper function to get message and user
async fn get_message_and_user(
    db: &SurrealDbClient,
    current_user: Option<User>,
    message_id: &str,
) -> Result<(Message, User, Conversation, Vec<Message>, Option<Message>), SseResponse> {
    // Check authentication
    let Some(user) = current_user else {
        return Err(sse_with_keep_alive(create_error_stream(
            "You must be signed in to use this feature",
        )));
    };

    // Retrieve message
    let message = match db.get_item::<Message>(message_id).await {
        Ok(Some(message)) => message,
        Ok(None) => {
            return Err(sse_with_keep_alive(create_error_stream(
                "Message not found: the specified message does not exist",
            )))
        }
        Err(e) => {
            error!("Database error retrieving message {}: {:?}", message_id, e);
            return Err(sse_with_keep_alive(create_error_stream(
                "Failed to retrieve message: database error",
            )));
        }
    };

    // Get conversation history
    let (conversation, history) =
        match Conversation::get_complete_conversation(&message.conversation_id, &user.id, db).await
        {
            Err(e) => {
                error!("Database error retrieving message {}: {:?}", message_id, e);
                return Err(sse_with_keep_alive(create_error_stream(
                    "Failed to retrieve message: database error",
                )));
            }
            Ok((conversation, history)) => (conversation, history),
        };

    let Some(message_index) = find_message_index(&history, message_id) else {
        return Err(sse_with_keep_alive(create_error_stream(
            "Message not found in conversation history",
        )));
    };

    let Some(message_from_history) = history.get(message_index) else {
        return Err(sse_with_keep_alive(create_error_stream(
            "Message not found in conversation history",
        )));
    };

    if message_from_history.role != MessageRole::User {
        return Err(sse_with_keep_alive(create_error_stream(
            "Only user messages can be used to generate a response",
        )));
    }

    let message = message_from_history.clone();

    let history_before_message = history_before_message(&history, message_index);
    let existing_ai_response = find_existing_ai_response(&history, message_index);

    Ok((
        message,
        user,
        conversation,
        history_before_message,
        existing_ai_response,
    ))
}

fn find_message_index(messages: &[Message], message_id: &str) -> Option<usize> {
    messages.iter().position(|message| message.id == message_id)
}

fn find_existing_ai_response(messages: &[Message], user_message_index: usize) -> Option<Message> {
    messages
        .iter()
        .skip(user_message_index + 1)
        .take_while(|message| message.role != MessageRole::User)
        .find(|message| message.role == MessageRole::AI)
        .cloned()
}

fn history_before_message(messages: &[Message], message_index: usize) -> Vec<Message> {
    messages.iter().take(message_index).cloned().collect()
}

fn create_replayed_response_stream(state: &HtmlState, existing_ai_message: Message) -> SseResponse {
    let references_event = if existing_ai_message
        .references
        .as_ref()
        .is_some_and(|references| !references.is_empty())
    {
        state
            .templates
            .render(
                "chat/reference_list.html",
                &Value::from_serialize(ReferenceData {
                    message: existing_ai_message.clone(),
                }),
            )
            .ok()
            .map(|html| Event::default().event("references").data(html))
    } else {
        None
    };

    let answer = existing_ai_message.content;

    let event_stream = stream! {
        yield Ok(Event::default().event("chat_message").data(answer));

        if let Some(event) = references_event {
            yield Ok(event);
        }

        yield Ok(Event::default().event("close_stream").data("Stream complete"));
    };

    sse_with_keep_alive(event_stream.boxed())
}

#[derive(Deserialize)]
pub struct QueryParams {
    message_id: String,
}

#[derive(Serialize)]
struct ReferenceData {
    message: Message,
}

fn extract_reference_strings(response: &LLMResponseFormat) -> Vec<String> {
    response
        .references
        .iter()
        .map(|reference| reference.reference.clone())
        .collect()
}

#[allow(clippy::too_many_lines)]
pub async fn get_response_stream(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
    Query(params): Query<QueryParams>,
) -> SseResponse {
    // 1. Authentication and initial data validation
    let (user_message, user, _conversation, history, existing_ai_response) =
        match get_message_and_user(&state.db, auth.current_user, &params.message_id).await {
            Ok((user_message, user, conversation, history, existing_ai_response)) => (
                user_message,
                user,
                conversation,
                history,
                existing_ai_response,
            ),
            Err(error_stream) => return error_stream,
        };

    if let Some(existing_ai_message) = existing_ai_response {
        return create_replayed_response_stream(&state, existing_ai_message);
    }

    // 2. Retrieve knowledge entities
    let rerank_lease = match state.reranker_pool.as_ref() {
        Some(pool) => Some(pool.checkout().await),
        None => None,
    };

    let strategy = state.retrieval_strategy();
    let config = retrieval_pipeline::RetrievalConfig::for_chat(strategy);

    let retrieval_result = match retrieval_pipeline::retrieve_entities(
        &state.db,
        &state.openai_client,
        Some(&*state.embedding_provider),
        &user_message.content,
        &user.id,
        config,
        rerank_lease,
    )
    .await
    {
        Ok(result) => result,
        Err(_e) => {
            return sse_with_keep_alive(create_error_stream("Failed to retrieve knowledge"));
        }
    };

    let allowed_reference_ids = collect_reference_ids_from_retrieval(&retrieval_result);

    // 3. Create the OpenAI request with appropriate context format
    let context_json = match &retrieval_result {
        retrieval_pipeline::StrategyOutput::Chunks(chunks) => chunks_to_chat_context(chunks),
        retrieval_pipeline::StrategyOutput::Entities(entities) => {
            retrieved_entities_to_json(entities)
        }
        retrieval_pipeline::StrategyOutput::Search(search_result) => {
            // For chat, use chunks from the search result
            chunks_to_chat_context(&search_result.chunks)
        }
    };
    let formatted_user_message =
        create_user_message_with_history(&context_json, &history, &user_message.content);
    let Ok(settings) = SystemSettings::get_current(&state.db).await else {
        return sse_with_keep_alive(create_error_stream("Failed to retrieve system settings"));
    };
    let Ok(request) = create_chat_request(formatted_user_message, &settings) else {
        return sse_with_keep_alive(create_error_stream("Failed to create chat request"));
    };

    // 4. Set up the OpenAI stream
    let openai_stream = match state.openai_client.chat().create_stream(request).await {
        Ok(stream) => stream,
        Err(_e) => {
            return sse_with_keep_alive(create_error_stream("Failed to create OpenAI stream"));
        }
    };

    // 5. Create channel for collecting complete response
    let (tx, mut rx) = channel::<String>(1000);
    let tx_clone = tx.clone();
    let (tx_final, mut rx_final) = channel::<Message>(1);

    // 6. Set up the collection task for DB storage
    let db_client = Arc::clone(&state.db);
    let user_id = user.id.clone();
    let allowed_reference_ids = allowed_reference_ids.clone();
    tokio::spawn(async move {
        drop(tx); // Close sender when no longer needed

        // Collect full response
        let mut full_json = String::new();
        while let Some(chunk) = rx.recv().await {
            full_json.push_str(&chunk);
        }

        // Try to extract structured data
        if let Ok(response) = from_str::<LLMResponseFormat>(&full_json) {
            let raw_references = extract_reference_strings(&response);
            let answer = response.answer;

            let initial_validation = match validate_references(
                &user_id,
                raw_references,
                &allowed_reference_ids,
                &db_client,
            )
            .await
            {
                Ok(result) => result,
                Err(err) => {
                    error!(error = %err, "Reference validation failed, storing answer without references");
                    let ai_message = Message::new(
                        user_message.conversation_id,
                        MessageRole::AI,
                        answer,
                        Some(Vec::new()),
                    );

                    let _ = tx_final.send(ai_message.clone()).await;
                    if let Err(store_err) = db_client.store_item(ai_message).await {
                        error!(error = ?store_err, "Failed to store AI message after validation failure");
                    }
                    return;
                }
            };

            info!(
                total_refs = initial_validation.reason_stats.total,
                valid_refs = initial_validation.valid_refs.len(),
                invalid_refs = initial_validation.invalid_refs.len(),
                invalid_empty = initial_validation.reason_stats.empty,
                invalid_unsupported_prefix = initial_validation.reason_stats.unsupported_prefix,
                invalid_malformed_uuid = initial_validation.reason_stats.malformed_uuid,
                invalid_duplicate = initial_validation.reason_stats.duplicate,
                invalid_not_in_context = initial_validation.reason_stats.not_in_context,
                invalid_not_found = initial_validation.reason_stats.not_found,
                invalid_wrong_user = initial_validation.reason_stats.wrong_user,
                invalid_over_limit = initial_validation.reason_stats.over_limit,
                "Post-LLM reference validation complete"
            );

            let ai_message = Message::new(
                user_message.conversation_id,
                MessageRole::AI,
                answer,
                Some(initial_validation.valid_refs),
            );

            let _ = tx_final.send(ai_message.clone()).await;

            match db_client.store_item(ai_message).await {
                Ok(_) => debug!("Successfully stored AI message with references"),
                Err(e) => error!("Failed to store AI message: {:?}", e),
            }
        } else {
            error!("Failed to parse LLM response as structured format");

            // Fallback - store raw response
            let ai_message = Message::new(
                user_message.conversation_id,
                MessageRole::AI,
                full_json,
                None,
            );

            let _ = db_client.store_item(ai_message).await;
        }
    });

    // Create a shared state for tracking the JSON parsing
    let json_state = Arc::new(Mutex::new(StreamParserState::new()));

    // 7. Create the response event stream
    let event_stream = openai_stream
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        .map(move |result| {
            let tx_storage = tx_clone.clone();
            let json_state = Arc::clone(&json_state);

            stream! {
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
                                yield Ok(Event::default()
                                    .event("chat_message")
                                    .data(display_content));
                            }
                            // If display_content is empty, don't yield anything
                        }
                        // If content is empty, don't yield anything
                    }
                    Err(e) => {
                        yield Ok(Event::default()
                            .event("error")
                            .data(format!("Stream error: {e}")));
                    }
                }
            }
        })
        .flatten()
        .chain(stream::once(async move {
            if let Some(message) = rx_final.recv().await {
                // Don't send any event if references is empty
                if message
                    .references
                    .as_ref()
                    .is_some_and(std::vec::Vec::is_empty)
                {
                    return Ok(Event::default().event("empty")); // This event won't be sent
                }

                // Render template with references
                match state.templates.render(
                    "chat/reference_list.html",
                    &Value::from_serialize(ReferenceData { message }),
                ) {
                    Ok(html) => {
                        // Return the rendered HTML
                        Ok(Event::default().event("references").data(html))
                    }
                    Err(_) => {
                        // Handle template rendering error
                        Ok(Event::default()
                            .event("error")
                            .data("Failed to render references"))
                    }
                }
            } else {
                // Handle case where no references were received
                Ok(Event::default()
                    .event("error")
                    .data("Failed to retrieve references"))
            }
        }))
        .chain(once(async {
            Ok(Event::default()
                .event("close_stream")
                .data("Stream complete"))
        }));

    sse_with_keep_alive(event_stream.boxed())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, Utc};
    use common::storage::{
        db::SurrealDbClient,
        types::{conversation::Conversation, user::Theme},
    };
    use retrieval_pipeline::answer_retrieval::Reference;
    use uuid::Uuid;

    fn make_test_message(id: &str, role: MessageRole) -> Message {
        let mut message = Message::new(
            "conversation-1".to_string(),
            role,
            format!("content-{id}"),
            None,
        );
        message.id = id.to_string();
        message
    }

    fn make_test_user(id: &str) -> User {
        User {
            id: id.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            email: "test@example.com".to_string(),
            password: "password".to_string(),
            anonymous: false,
            api_key: None,
            admin: false,
            timezone: "UTC".to_string(),
            theme: Theme::System,
        }
    }

    #[test]
    fn extracts_reference_strings_in_order() {
        let response = LLMResponseFormat {
            answer: "answer".to_string(),
            references: vec![
                Reference {
                    reference: "a".to_string(),
                },
                Reference {
                    reference: "b".to_string(),
                },
            ],
        };

        let extracted = extract_reference_strings(&response);
        assert_eq!(extracted, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn finds_message_index_for_existing_message() {
        let messages = vec![
            make_test_message("m1", MessageRole::User),
            make_test_message("m2", MessageRole::AI),
            make_test_message("m3", MessageRole::User),
        ];

        assert_eq!(find_message_index(&messages, "m2"), Some(1));
        assert_eq!(find_message_index(&messages, "missing"), None);
    }

    #[test]
    fn finds_existing_ai_response_for_same_turn() {
        let messages = vec![
            make_test_message("u1", MessageRole::User),
            make_test_message("system", MessageRole::System),
            make_test_message("a1", MessageRole::AI),
            make_test_message("u2", MessageRole::User),
            make_test_message("a2", MessageRole::AI),
        ];

        let ai_reply = find_existing_ai_response(&messages, 0).expect("expected AI response");
        assert_eq!(ai_reply.id, "a1");

        let ai_reply_second_turn =
            find_existing_ai_response(&messages, 3).expect("expected AI response");
        assert_eq!(ai_reply_second_turn.id, "a2");
    }

    #[test]
    fn does_not_replay_ai_response_from_later_turn() {
        let messages = vec![
            make_test_message("u1", MessageRole::User),
            make_test_message("u2", MessageRole::User),
            make_test_message("a2", MessageRole::AI),
        ];

        assert!(find_existing_ai_response(&messages, 0).is_none());

        let ai_reply = find_existing_ai_response(&messages, 1).expect("expected AI response");
        assert_eq!(ai_reply.id, "a2");
    }

    #[test]
    fn history_before_message_excludes_target_and_future_messages() {
        let messages = vec![
            make_test_message("u1", MessageRole::User),
            make_test_message("a1", MessageRole::AI),
            make_test_message("u2", MessageRole::User),
            make_test_message("a2", MessageRole::AI),
        ];

        let history_for_u2 = history_before_message(&messages, 2);
        let history_ids: Vec<String> = history_for_u2
            .into_iter()
            .map(|message| message.id)
            .collect();
        assert_eq!(history_ids, vec!["u1".to_string(), "a1".to_string()]);
    }

    #[tokio::test]
    async fn get_message_and_user_reuses_existing_ai_response_for_same_turn() {
        let namespace = "chat_stream_replay";
        let database = Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, &database)
            .await
            .expect("failed to create in-memory db");

        let user = make_test_user("user-1");
        let conversation = Conversation::new(user.id.clone(), "Conversation".to_string());

        let mut user_message = Message::new(
            conversation.id.clone(),
            MessageRole::User,
            "Question one".to_string(),
            None,
        );
        user_message.id = "u1".to_string();

        let mut ai_message = Message::new(
            conversation.id.clone(),
            MessageRole::AI,
            "Answer one".to_string(),
            Some(vec!["ref-1".to_string()]),
        );
        ai_message.id = "a1".to_string();
        ai_message.created_at = user_message.created_at + ChronoDuration::seconds(1);
        ai_message.updated_at = ai_message.created_at;

        let mut second_user_message = Message::new(
            conversation.id.clone(),
            MessageRole::User,
            "Question two".to_string(),
            None,
        );
        second_user_message.id = "u2".to_string();
        second_user_message.created_at = ai_message.created_at + ChronoDuration::seconds(1);
        second_user_message.updated_at = second_user_message.created_at;

        db.store_item(conversation.clone())
            .await
            .expect("failed to store conversation");
        db.store_item(user_message.clone())
            .await
            .expect("failed to store user message");
        db.store_item(ai_message.clone())
            .await
            .expect("failed to store ai message");
        db.store_item(second_user_message.clone())
            .await
            .expect("failed to store second user message");

        let (_, _, _, history_for_first_turn, existing_ai_for_first_turn) =
            get_message_and_user(&db, Some(user.clone()), &user_message.id)
                .await
                .expect("expected first turn to load");

        assert!(history_for_first_turn.is_empty());
        let existing_ai_for_first_turn =
            existing_ai_for_first_turn.expect("expected first-turn AI response");
        assert_eq!(existing_ai_for_first_turn.id, ai_message.id);

        let (_, _, _, history_for_second_turn, existing_ai_for_second_turn) =
            get_message_and_user(&db, Some(user), &second_user_message.id)
                .await
                .expect("expected second turn to load");

        let history_ids: Vec<String> = history_for_second_turn
            .into_iter()
            .map(|message| message.id)
            .collect();
        assert_eq!(history_ids, vec!["u1".to_string(), "a1".to_string()]);
        assert!(existing_ai_for_second_turn.is_none());
    }
}
