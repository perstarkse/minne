mod bootstrap;

use std::sync::Arc;

use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::{
    storage::{
        indexes::ensure_runtime,
        types::{
            knowledge_entity::KnowledgeEntity, system_settings::SystemSettings,
            text_chunk::TextChunk,
        },
    },
};
use html_router::{
    html_routes,
    html_state::{HtmlState, StateResources},
};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use tracing::{error, info, warn};
use tokio::task::LocalSet;

fn spawn_server_thread(
    listener: tokio::net::TcpListener,
    app: Router,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                error!("Failed to create server runtime: {e}");
                return;
            }
        };
        rt.block_on(async {
            if let Err(e) = axum::serve(listener, app).await {
                error!("Server error: {}", e);
            }
        });
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let services = bootstrap::init().await?;

    let session_store = Arc::new(services.db.create_session_store().await?);

    info!(
        embedding_backend = ?services.config.embedding_backend,
        embedding_dimension = services.embedding_provider.dimension(),
        "Embedding provider initialized"
    );

    let (settings, dimensions_changed) =
        SystemSettings::sync_from_embedding_provider(&services.db, &services.embedding_provider)
            .await?;

    if dimensions_changed {
        warn!(
            new_dimensions = settings.embedding_dimensions,
            "Embedding configuration changed; re-embedding existing data"
        );

        info!("Re-embedding TextChunks");
        if let Err(e) =
            TextChunk::update_all_embeddings_with_provider(&services.db, &services.embedding_provider)
                .await
        {
            error!(
                "Failed to re-embed TextChunks: {}. Search results may be stale.",
                e
            );
        }

        info!("Re-embedding KnowledgeEntities");
        if let Err(e) =
            KnowledgeEntity::update_all_embeddings_with_provider(&services.db, &services.embedding_provider)
                .await
        {
            error!(
                "Failed to re-embed KnowledgeEntities: {}. Search results may be stale.",
                e
            );
        }

        info!("Re-embedding complete.");
    }

    ensure_runtime(&services.db, settings.embedding_dimensions as usize).await?;

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

    let api_state = ApiState::new(&services.config, services.storage.clone()).await?;

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

    let server_handle = spawn_server_thread(listener, app);

    let ingestion_pipeline = Arc::new(IngestionPipeline::new(
        Arc::clone(&services.db),
        Arc::clone(&services.openai_client),
        services.config.clone(),
        services.reranker_pool.clone(),
        services.storage,
        Arc::clone(&services.embedding_provider),
    )?);

    let local = LocalSet::new();
    local.spawn_local(async move {
        info!("Starting worker process");
        if let Err(e) = run_worker_loop(services.db, ingestion_pipeline).await {
            error!("Worker error: {}", e);
        }
    });
    local.await;

    if let Err(e) = server_handle.join() {
        error!("Server thread panicked: {:?}", e);
    }

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: ApiState,
    html_state: HtmlState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        response::Response,
        Router,
    };
    use common::storage::{
        db::SurrealDbClient,
        store::StorageManager,
        types::{system_settings::SystemSettings, user::User},
    };
    use common::utils::config::{AppConfig, EmbeddingBackend, PdfIngestMode, StorageKind};
    use std::{path::Path, sync::Arc};
    use tower::ServiceExt;
    use uuid::Uuid;

    fn smoke_test_config(namespace: &str, database: &str, data_dir: &Path) -> AppConfig {
        AppConfig {
            openai_api_key: "test-key".into(),
            surrealdb_address: "mem://".into(),
            surrealdb_username: "root".into(),
            surrealdb_password: "root".into(),
            surrealdb_namespace: namespace.into(),
            surrealdb_database: database.into(),
            data_dir: data_dir.to_string_lossy().into_owned(),
            http_port: 0,
            openai_base_url: "https://example.com".into(),
            storage: StorageKind::Local,
            pdf_ingest_mode: PdfIngestMode::LlmFirst,
            embedding_backend: EmbeddingBackend::Hashed,
            ..Default::default()
        }
    }

    async fn build_test_app() -> (Router, Arc<SurrealDbClient>, std::path::PathBuf) {
        let namespace = "test_ns";
        let database = format!("test_db_{}", Uuid::new_v4());
        let data_dir = std::env::temp_dir().join(format!("minne_smoke_{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&data_dir).await
            .expect("failed to create temp data directory");

        let config = smoke_test_config(namespace, &database, &data_dir);
        let services = crate::bootstrap::init_with_config(config.clone())
            .await
            .expect("failed to init services");

        let session_store = Arc::new(
            services
                .db
                .create_session_store()
                .await
                .expect("failed to create session store"),
        );

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

        let api_state = ApiState {
            db: Arc::clone(&services.db),
            config: services.config.clone(),
            storage: services.storage,
        };

        let app = Router::new()
            .nest("/api/v1", api_routes_v1(&api_state))
            .merge(html_routes(&html_state))
            .with_state(AppState {
                api_state,
                html_state,
            });

        (app, services.db, data_dir)
    }

    fn assert_redirect_to(response: &Response, expected_location: &str) {
        assert!(response.status().is_redirection());
        let location = response
            .headers()
            .get(header::LOCATION)
            .expect("redirect should contain a Location header")
            .to_str()
            .expect("location header must be valid utf-8");
        assert_eq!(location, expected_location);
    }

    fn extract_session_cookie(response: &Response) -> String {
        let cookie_header = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .expect("set-cookie header must be valid utf-8")
                    .split(';')
                    .next()
                    .expect("set-cookie should include key=value pair")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert!(
            !cookie_header.is_empty(),
            "login response should set at least one cookie"
        );

        cookie_header.join("; ")
    }

    async fn sign_in_and_get_cookie(app: &Router, email: &str, password: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/signin")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("email={email}&password={password}")))
                    .expect("signin request"),
            )
            .await
            .expect("signin response");

        assert_redirect_to(&response, "/");
        extract_session_cookie(&response)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn smoke_startup_with_in_memory_surrealdb() {
        let (app, _db, data_dir) = build_test_app().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/live")
                    .body(Body::empty())
                    .expect("building live request"),
            )
            .await
            .expect("sending live request");
        assert_eq!(response.status(), StatusCode::OK);

        let ready_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ready")
                    .body(Body::empty())
                    .expect("building ready request"),
            )
            .await
            .expect("sending ready request");
        assert_eq!(ready_response.status(), StatusCode::OK);

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admin_route_enforces_unauth_non_admin_and_admin_access() {
        let (app, db, data_dir) = build_test_app().await;

        let admin = User::create_new(
            "admin_user".to_string(),
            "admin_password".to_string(),
            &db,
            "UTC".to_string(),
            "system".to_string(),
        )
        .await
        .expect("admin user should be created");
        let non_admin = User::create_new(
            "member_user".to_string(),
            "member_password".to_string(),
            &db,
            "UTC".to_string(),
            "system".to_string(),
        )
        .await
        .expect("non-admin user should be created");

        assert!(admin.admin, "first user should become admin");
        assert!(!non_admin.admin, "second user should not be admin");

        let unauth_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .expect("unauth admin request"),
            )
            .await
            .expect("unauth admin response");
        assert_redirect_to(&unauth_response, "/signin");

        let non_admin_cookie = sign_in_and_get_cookie(&app, "member_user", "member_password").await;
        let non_admin_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .header(header::COOKIE, non_admin_cookie)
                    .body(Body::empty())
                    .expect("non-admin request"),
            )
            .await
            .expect("non-admin response");
        assert_redirect_to(&non_admin_response, "/");

        let admin_cookie = sign_in_and_get_cookie(&app, "admin_user", "admin_password").await;
        let admin_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .header(header::COOKIE, admin_cookie)
                    .body(Body::empty())
                    .expect("admin request"),
            )
            .await
            .expect("admin response");
        assert_eq!(admin_response.status(), StatusCode::OK);

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admin_patch_blocks_non_admin_and_unauth_before_side_effects() {
        let (app, db, data_dir) = build_test_app().await;

        User::create_new(
            "admin_user_patch".to_string(),
            "admin_password_patch".to_string(),
            &db,
            "UTC".to_string(),
            "system".to_string(),
        )
        .await
        .expect("admin user should be created");
        User::create_new(
            "member_user_patch".to_string(),
            "member_password_patch".to_string(),
            &db,
            "UTC".to_string(),
            "system".to_string(),
        )
        .await
        .expect("non-admin user should be created");

        let initial_settings = SystemSettings::get_current(&db)
            .await
            .expect("settings should be available");

        let patch_body = if initial_settings.registrations_enabled {
            String::new()
        } else {
            "registration_open=on".to_string()
        };
        let expected_after_admin_patch = !initial_settings.registrations_enabled;

        let unauth_patch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/toggle-registrations")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(patch_body.clone()))
                    .expect("unauth patch request"),
            )
            .await
            .expect("unauth patch response");
        assert_redirect_to(&unauth_patch_response, "/signin");

        let settings_after_unauth = SystemSettings::get_current(&db)
            .await
            .expect("settings should still be available");
        assert_eq!(
            settings_after_unauth.registrations_enabled,
            initial_settings.registrations_enabled
        );

        let non_admin_cookie =
            sign_in_and_get_cookie(&app, "member_user_patch", "member_password_patch").await;
        let non_admin_patch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/toggle-registrations")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(header::COOKIE, non_admin_cookie)
                    .body(Body::from(patch_body.clone()))
                    .expect("non-admin patch request"),
            )
            .await
            .expect("non-admin patch response");
        assert_redirect_to(&non_admin_patch_response, "/");

        let settings_after_non_admin = SystemSettings::get_current(&db)
            .await
            .expect("settings should still be available");
        assert_eq!(
            settings_after_non_admin.registrations_enabled,
            initial_settings.registrations_enabled
        );

        let admin_cookie =
            sign_in_and_get_cookie(&app, "admin_user_patch", "admin_password_patch").await;
        let admin_patch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/toggle-registrations")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(header::COOKIE, admin_cookie)
                    .body(Body::from(patch_body))
                    .expect("admin patch request"),
            )
            .await
            .expect("admin patch response");
        assert_eq!(admin_patch_response.status(), StatusCode::OK);

        let settings_after_admin = SystemSettings::get_current(&db)
            .await
            .expect("settings should still be available");
        assert_eq!(
            settings_after_admin.registrations_enabled,
            expected_after_admin_patch
        );

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }
}
