mod bootstrap;

use std::sync::Arc;

use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::storage::types::system_settings::SystemSettings;
use html_router::{
    html_routes,
    html_state::{HtmlState, StateResources},
};
use tracing::info;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let services = bootstrap::init().await?;

    let session_store = Arc::new(services.db.create_session_store().await?);

    let (_settings, _dimensions_changed) =
        SystemSettings::sync_from_embedding_provider(&services.db, &services.embedding_provider)
            .await?;

    let html_state = HtmlState::new_with_resources(StateResources {
        db: Arc::clone(&services.db),
        openai_client: Arc::clone(&services.openai_client),
        session_store,
        storage: services.storage.clone(),
        config: services.config.clone(),
        reranker_pool: services.reranker_pool.clone(),
        embedding_provider: Arc::clone(&services.embedding_provider),
        template_engine: None,
    });

    let api_state = ApiState::new(&services.config, services.storage).await?;

    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&api_state))
        .merge(html_routes(&html_state))
        .with_state(AppState {
            api_state,
            html_state,
        });

    info!(
        "Starting server listening on 0.0.0.0:{}",
        services.config.http_port
    );
    let serve_address = format!("0.0.0.0:{}", services.config.http_port);
    let listener = tokio::net::TcpListener::bind(serve_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: ApiState,
    html_state: HtmlState,
}
