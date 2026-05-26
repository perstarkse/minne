use api_router::{api_routes_v1, api_state::ApiState};
use axum::{extract::FromRef, Router};
use common::{
    storage::{
        db::SurrealDbClient,
        indexes::ensure_runtime,
        store::StorageManager,
        types::{
            knowledge_entity::KnowledgeEntity, system_settings::SystemSettings,
            text_chunk::TextChunk,
        },
    },
    utils::{config::get_config, embedding::EmbeddingProvider},
};
use html_router::{
    html_routes,
    html_state::{HtmlState, StateResources},
};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use retrieval_pipeline::reranking::RerankerPool;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

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

async fn run_worker(
    config: common::utils::config::AppConfig,
    reranker_pool: Option<Arc<RerankerPool>>,
    storage: StorageManager,
) -> anyhow::Result<()> {
    let worker_db = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    );

    let openai_client = Arc::new(async_openai::Client::with_config(
        async_openai::config::OpenAIConfig::new()
            .with_api_key(&config.openai_api_key)
            .with_api_base(&config.openai_base_url),
    ));

    let embedding_provider = Arc::new(
        EmbeddingProvider::from_config(&config, Some(Arc::clone(&openai_client))).await?,
    );

    let ingestion_pipeline = Arc::new(
        IngestionPipeline::new(
            Arc::clone(&worker_db),
            openai_client,
            config,
            reranker_pool,
            storage,
            embedding_provider,
        )?,
    );

    info!("Starting worker process");
    run_worker_loop(worker_db, ingestion_pipeline).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;

    let db = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    );

    db.apply_migrations().await?;

    let session_store = Arc::new(db.create_session_store().await?);
    let openai_client = Arc::new(async_openai::Client::with_config(
        async_openai::config::OpenAIConfig::new()
            .with_api_key(&config.openai_api_key)
            .with_api_base(&config.openai_base_url),
    ));

    let embedding_provider =
        Arc::new(EmbeddingProvider::from_config(&config, Some(Arc::clone(&openai_client))).await?);
    info!(
        embedding_backend = ?config.embedding_backend,
        embedding_dimension = embedding_provider.dimension(),
        "Embedding provider initialized"
    );

    let (settings, dimensions_changed) =
        SystemSettings::sync_from_embedding_provider(&db, &embedding_provider).await?;

    if dimensions_changed {
        warn!(
            new_dimensions = settings.embedding_dimensions,
            "Embedding configuration changed; re-embedding existing data"
        );

        info!("Re-embedding TextChunks");
        if let Err(e) =
            TextChunk::update_all_embeddings_with_provider(&db, &embedding_provider).await
        {
            error!(
                "Failed to re-embed TextChunks: {}. Search results may be stale.",
                e
            );
        }

        info!("Re-embedding KnowledgeEntities");
        if let Err(e) =
            KnowledgeEntity::update_all_embeddings_with_provider(&db, &embedding_provider).await
        {
            error!(
                "Failed to re-embed KnowledgeEntities: {}. Search results may be stale.",
                e
            );
        }

        info!("Re-embedding complete.");
    }

    ensure_runtime(&db, settings.embedding_dimensions as usize).await?;

    let reranker_pool = RerankerPool::maybe_from_config(&config)?;

    let storage = StorageManager::new(&config).await?;

        let html_state = HtmlState::new_with_resources(StateResources {
        db,
        openai_client,
        session_store,
        storage: storage.clone(),
        config: config.clone(),
        reranker_pool: reranker_pool.clone(),
        embedding_provider: Arc::clone(&embedding_provider),
        template_engine: None,
    });

    let api_state = ApiState::new(&config, storage.clone()).await?;

    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&api_state))
        .merge(html_routes(&html_state))
        .with_state(AppState {
            api_state,
            html_state,
        });

    info!("Starting server listening on 0.0.0.0:{}", config.http_port);
    let serve_address = format!("0.0.0.0:{}", config.http_port);
    let listener = tokio::net::TcpListener::bind(serve_address).await?;

    let server_handle = spawn_server_thread(listener, app);

    let local = LocalSet::new();
    local.spawn_local(async move {
        if let Err(e) = run_worker(config, reranker_pool, storage).await {
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
        store::StorageManager,
        types::{system_settings::SystemSettings, user::User},
    };
    use common::utils::config::{AppConfig, PdfIngestMode, StorageKind};
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
        let db = Arc::new(SurrealDbClient::memory(namespace, &database).await?);
        db.apply_migrations().await?;

        let session_store = Arc::new(db.create_session_store().await?);
        let openai_client = Arc::new(async_openai::Client::with_config(
            async_openai::config::OpenAIConfig::new()
                .with_api_key(&config.openai_api_key)
                .with_api_base(&config.openai_base_url),
        ));

        let storage = StorageManager::new(&config).await?;

        let embedding_provider = Arc::new(
            common::utils::embedding::EmbeddingProvider::new_hashed(384)?,
        );

    let html_state = HtmlState::new_with_resources(StateResources {
            db: Arc::clone(&db),
            openai_client,
            session_store,
            storage: storage.clone(),
            config: config.clone(),
            reranker_pool: None,
            embedding_provider,
            template_engine: None,
        });

        let api_state = ApiState {
            db: Arc::clone(&db),
            config: config.clone(),
            storage,
        };

        let app = Router::new()
            .nest("/api/v1", api_routes_v1(&api_state))
            .merge(html_routes(&html_state))
            .with_state(AppState {
                api_state,
                html_state,
            });

        (app, db, data_dir)
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
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);

        let ready_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ready")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(ready_response.status(), StatusCode::OK);

        tokio::fs::remove_dir_all(&data_dir).await.ok();
        Ok(())
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
