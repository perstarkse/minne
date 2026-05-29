use std::sync::Arc;

use anyhow::Context;
use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use html_router::{
    html_routes,
    html_state::{HtmlState, StateResources},
};

use super::SharedServices;

/// Builds the Minne API and HTML route subtrees without fixing the outer Axum state
/// type. SaaS consumers can merge additional routers and attach their own `AppState`
/// as long as it implements `FromRef` for `ApiState` and `HtmlState`.
pub fn minne_routes<S>(api_state: &ApiState, html_state: &HtmlState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ApiState: FromRef<S>,
    HtmlState: FromRef<S>,
{
    Router::new()
        .nest("/api/v1", api_routes_v1(api_state))
        .merge(html_routes(html_state))
}

pub fn build_api_state(services: &SharedServices) -> ApiState {
    ApiState {
        db: Arc::clone(&services.db),
        config: services.config.clone(),
        storage: services.storage.clone(),
    }
}

pub async fn build_html_state(services: &SharedServices) -> anyhow::Result<HtmlState> {
    let session_store = Arc::new(
        services
            .db
            .create_session_store()
            .await
            .context("create session store")?,
    );

    Ok(HtmlState::new_with_resources(StateResources {
        db: Arc::clone(&services.db),
        openai_client: Arc::clone(&services.openai_client),
        session_store,
        storage: services.storage.clone(),
        config: services.config.clone(),
        reranker_pool: services.reranker_pool.clone(),
        embedding_provider: Arc::clone(&services.embedding_provider),
        template_engine: None,
    }))
}
