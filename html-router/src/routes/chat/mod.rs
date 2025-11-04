mod chat_handlers;
mod message_response_stream;
mod references;

use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
pub use chat_handlers::{
    delete_conversation, new_chat_user_message, new_user_message, patch_conversation_title,
    reload_sidebar, show_chat_base, show_conversation_editing_title, show_existing_chat,
    show_initialized_chat,
};
use message_response_stream::get_response_stream;
use references::show_reference_tooltip;

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/chat", get(show_chat_base).post(new_chat_user_message))
        .route(
            "/chat/{id}",
            get(show_existing_chat)
                .post(new_user_message)
                .delete(delete_conversation),
        )
        .route(
            "/chat/{id}/title",
            get(show_conversation_editing_title).patch(patch_conversation_title),
        )
        .route("/chat/sidebar", get(reload_sidebar))
        .route("/initialized-chat", post(show_initialized_chat))
        .route("/chat/response-stream", get(get_response_stream))
        .route("/chat/reference/{id}", get(show_reference_tooltip))
}
