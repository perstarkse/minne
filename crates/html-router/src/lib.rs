pub mod html_state;
mod middleware_analytics;
mod middleware_auth;
mod routes;
mod template_response;

use axum::{
    extract::FromRef,
    middleware::{from_fn_with_state, map_response_with_state},
    routing::{delete, get, patch, post},
    Router,
};
use axum_session::{Session, SessionLayer};
use axum_session_auth::{AuthConfig, AuthSession, AuthSessionLayer};
use axum_session_surreal::SessionSurrealPool;
use common::storage::types::user::User;
use html_state::HtmlState;
use middleware_analytics::analytics_middleware;
use middleware_auth::require_auth;
use routes::{
    account::{
        change_password, delete_account, set_api_key, show_account_page, show_change_password,
        update_timezone,
    },
    admin_panel::{
        patch_ingestion_prompt, patch_query_prompt, show_admin_panel, show_edit_ingestion_prompt,
        show_edit_system_prompt, toggle_registration_status, update_model_settings,
    },
    chat::{
        message_response_stream::get_response_stream, new_chat_user_message, new_user_message,
        references::show_reference_tooltip, show_chat_base, show_existing_chat,
        show_initialized_chat,
    },
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
use template_response::with_template_response;
use tower_http::services::ServeDir;

pub type AuthSessionType = AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>;
pub type SessionType = Session<SessionSurrealPool<Any>>;

/// Html routes
pub fn html_routes<S>(app_state: &HtmlState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    // Public routes - no auth required
    let public_routes = Router::new()
        .route("/", get(index_handler))
        .route("/gdpr/accept", post(accept_gdpr))
        .route("/gdpr/deny", post(deny_gdpr))
        .route("/signout", get(sign_out_user))
        .route("/signin", get(show_signin_form).post(authenticate_user))
        .route(
            "/signup",
            get(show_signup_form).post(process_signup_and_show_verification),
        )
        .route("/documentation", get(show_documentation_index))
        .route("/documentation/privacy-policy", get(show_privacy_policy))
        .route("/documentation/get-started", get(show_get_started))
        .route("/documentation/mobile-friendly", get(show_mobile_friendly))
        .nest_service("/assets", ServeDir::new("assets/"));

    // Protected routes - auth required
    let protected_routes = Router::new()
        .route("/chat", get(show_chat_base).post(new_chat_user_message))
        .route("/initialized-chat", post(show_initialized_chat))
        .route("/chat/:id", get(show_existing_chat).post(new_user_message))
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
        .route("/search", get(search_result_handler))
        .route("/chat/response-stream", get(get_response_stream))
        .route("/knowledge/:id", get(show_reference_tooltip))
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
        // Admin page
        .route("/admin", get(show_admin_panel))
        .route("/toggle-registrations", patch(toggle_registration_status))
        .route("/update-model-settings", patch(update_model_settings))
        .route("/edit-query-prompt", get(show_edit_system_prompt))
        .route("/update-query-prompt", patch(patch_query_prompt))
        .route("/edit-ingestion-prompt", get(show_edit_ingestion_prompt))
        .route("/update-ingestion-prompt", patch(patch_ingestion_prompt))
        // User account page
        .route("/account", get(show_account_page))
        .route("/set-api-key", post(set_api_key))
        .route("/update-timezone", patch(update_timezone))
        .route(
            "/change-password",
            get(show_change_password).patch(change_password),
        )
        .route("/delete-account", delete(delete_account))
        .route_layer(from_fn_with_state(app_state.clone(), require_auth));

    // Combine routes and add common middleware
    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .layer(from_fn_with_state(app_state.clone(), analytics_middleware))
        .layer(map_response_with_state(
            app_state.clone(),
            with_template_response,
        ))
        .layer(
            AuthSessionLayer::<User, String, SessionSurrealPool<Any>, Surreal<Any>>::new(Some(
                app_state.db.client.clone(),
            ))
            .with_config(AuthConfig::<String>::default()),
        )
        .layer(SessionLayer::new((*app_state.session_store).clone()))
}
