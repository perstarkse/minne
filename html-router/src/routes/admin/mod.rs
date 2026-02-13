mod handlers;
use axum::{
    extract::FromRef,
    middleware::from_fn,
    routing::{get, patch},
    Router,
};
use handlers::{
    patch_image_prompt, patch_ingestion_prompt, patch_query_prompt, show_admin_panel,
    show_edit_image_prompt, show_edit_ingestion_prompt, show_edit_system_prompt,
    toggle_registration_status, update_model_settings,
};

use crate::{html_state::HtmlState, middlewares::auth_middleware::require_admin};

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    Router::new()
        .route("/admin", get(show_admin_panel))
        .route("/toggle-registrations", patch(toggle_registration_status))
        .route("/update-model-settings", patch(update_model_settings))
        .route("/edit-query-prompt", get(show_edit_system_prompt))
        .route("/update-query-prompt", patch(patch_query_prompt))
        .route("/edit-ingestion-prompt", get(show_edit_ingestion_prompt))
        .route("/update-ingestion-prompt", patch(patch_ingestion_prompt))
        .route("/edit-image-prompt", get(show_edit_image_prompt))
        .route("/update-image-prompt", patch(patch_image_prompt))
        .route_layer(from_fn(require_admin))
}
