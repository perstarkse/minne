mod handlers;

use axum::{
    extract::FromRef,
    routing::{delete, get, post},
    Router,
};
use handlers::{
    create_knowledge_entity, delete_knowledge_entity, delete_knowledge_relationship,
    get_knowledge_graph_json, patch_knowledge_entity, save_knowledge_relationship,
    show_edit_knowledge_entity_form, show_knowledge_page, show_new_knowledge_entity_form,
    suggest_knowledge_relationships,
};

use crate::html_state::HtmlState;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/knowledge", get(show_knowledge_page))
        .route("/knowledge/graph.json", get(get_knowledge_graph_json))
        .route("/knowledge-entity/new", get(show_new_knowledge_entity_form))
        .route("/knowledge-entity", post(create_knowledge_entity))
        .route(
            "/knowledge-entity/{id}",
            get(show_edit_knowledge_entity_form)
                .delete(delete_knowledge_entity)
                .patch(patch_knowledge_entity),
        )
        .route(
            "/knowledge-entity/suggestions",
            post(suggest_knowledge_relationships),
        )
        .route("/knowledge-relationship", post(save_knowledge_relationship))
        .route(
            "/knowledge-relationship/{id}",
            delete(delete_knowledge_relationship),
        )
}
