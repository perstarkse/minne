mod handlers;

use axum::{extract::FromRef, routing::get, Router};
use handlers::{
    delete_text_content, patch_text_content, show_content_page, show_text_content_edit_form,
};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/content", get(show_content_page))
        .route(
            "/content/{id}",
            get(show_text_content_edit_form)
                .patch(patch_text_content)
                .delete(delete_text_content),
        )
}
