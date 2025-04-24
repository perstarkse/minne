mod handlers;

use axum::{
    extract::FromRef,
    routing::{delete, get, post},
    Router,
};
use handlers::{
    delete_knowledge_entity, delete_knowledge_relationship, patch_knowledge_entity,
    save_knowledge_relationship, show_edit_knowledge_entity_form, show_knowledge_page,
};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/knowledge", get(show_knowledge_page))
        .route(
            "/knowledge-entity/{id}",
            get(show_edit_knowledge_entity_form)
                .delete(delete_knowledge_entity)
                .patch(patch_knowledge_entity),
        )
        .route("/knowledge-relationship", post(save_knowledge_relationship))
        .route(
            "/knowledge-relationship/{id}",
            delete(delete_knowledge_relationship),
        )
}
