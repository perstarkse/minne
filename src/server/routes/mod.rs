use api::{
    ingress::ingress_data,
    ingress_task::{delete_queue_task, get_queue_tasks},
    query::query_handler,
    queue_length::queue_length_handler,
};
use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post},
    Router,
};
use axum_session::{SessionLayer, SessionStore};
use axum_session_auth::{AuthConfig, AuthSessionLayer};
use axum_session_surreal::SessionSurrealPool;
use html::{
    account::{delete_account, set_api_key, show_account_page, update_timezone},
    admin_panel::{show_admin_panel, toggle_registration_status},
    content::{patch_text_content, show_content_page, show_text_content_edit_form},
    documentation::{
        show_documentation_index, show_get_started, show_mobile_friendly, show_privacy_policy,
    },
    gdpr::{accept_gdpr, deny_gdpr},
    index::{delete_job, delete_text_content, index_handler, show_active_jobs},
    ingress_form::{hide_ingress_form, process_ingress_form, show_ingress_form},
    knowledge::{
        delete_knowledge_entity, delete_knowledge_relationship, patch_knowledge_entity,
        save_knowledge_relationship, show_edit_knowledge_entity_form, show_knowledge_page,
    },
    search_result::search_result_handler,
    signin::{authenticate_user, show_signin_form},
    signout::sign_out_user,
    signup::{process_signup_and_show_verification, show_signup_form},
};
use surrealdb::{engine::any::Any, Surreal};
use tower_http::services::ServeDir;

use crate::storage::types::user::User;

use super::{middleware_analytics::analytics_middleware, middleware_api_auth::api_auth, AppState};

pub mod api;
pub mod html;

/// Router for API functionality, version 1
pub fn api_routes_v1(app_state: &AppState) -> Router<AppState> {
    Router::new()
        // Ingress routes
        .route("/ingress", post(ingress_data))
        .route("/message_count", get(queue_length_handler))
        .route("/queue", get(get_queue_tasks))
        .route("/queue/:delivery_tag", delete(delete_queue_task))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        // Query routes
        .route("/query", post(query_handler))
        .route_layer(from_fn_with_state(app_state.clone(), api_auth))
}

/// Router for HTML endpoints
pub fn html_routes(app_state: &AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(index_handler))
        .route("/gdpr/accept", post(accept_gdpr))
        .route("/gdpr/deny", post(deny_gdpr))
        .route("/search", get(search_result_handler))
        .route("/signout", get(sign_out_user))
        .route("/signin", get(show_signin_form).post(authenticate_user))
        .route(
            "/ingress-form",
            get(show_ingress_form).post(process_ingress_form),
        )
        .route("/hide-ingress-form", get(hide_ingress_form))
        .route("/text-content/:id", delete(delete_text_content))
        .route("/jobs/:job_id", delete(delete_job))
        .route("/active-jobs", get(show_active_jobs))
        .route("/content", get(show_content_page))
        .route(
            "/content/:id",
            get(show_text_content_edit_form).patch(patch_text_content),
        )
        .route("/knowledge", get(show_knowledge_page))
        .route(
            "/knowledge-entity/:id",
            get(show_edit_knowledge_entity_form)
                .delete(delete_knowledge_entity)
                .patch(patch_knowledge_entity),
        )
        .route("/knowledge-relationship", post(save_knowledge_relationship))
        .route(
            "/knowledge-relationship/:id",
            delete(delete_knowledge_relationship),
        )
        .route("/account", get(show_account_page))
        .route("/admin", get(show_admin_panel))
        .route("/toggle-registrations", patch(toggle_registration_status))
        .route("/set-api-key", post(set_api_key))
        .route("/update-timezone", patch(update_timezone))
        .route("/delete-account", delete(delete_account))
        .route(
            "/signup",
            get(show_signup_form).post(process_signup_and_show_verification),
        )
        .route("/documentation", get(show_documentation_index))
        .route("/documentation/privacy-policy", get(show_privacy_policy))
        .route("/documentation/get-started", get(show_get_started))
        .route("/documentation/mobile-friendly", get(show_mobile_friendly))
        .nest_service("/assets", ServeDir::new("assets/"))
        .layer(from_fn_with_state(app_state.clone(), analytics_middleware))
        .layer(
            AuthSessionLayer::<User, String, SessionSurrealPool<Any>, Surreal<Any>>::new(Some(
                app_state.surreal_db_client.client.clone(),
            ))
            .with_config(AuthConfig::<String>::default()),
        )
        .layer(SessionLayer::new((*app_state.session_store).clone()))
}
