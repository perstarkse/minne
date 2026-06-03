mod bootstrap;

use std::sync::Arc;

use axum::extract::FromRef;
use bootstrap::{
    init, prepare_embedding_runtime,
    wiring::{build_api_state, build_html_state, minne_routes},
    EmbeddingRuntimeRole,
};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let services = init().await?;

    info!(
        embedding_backend = ?services.config.embedding_backend,
        embedding_dimension = services.embedding_provider.dimension(),
        "Embedding provider initialized"
    );

    // The combined binary runs the worker in-process, so it owns re-embedding.
    prepare_embedding_runtime(&services, EmbeddingRuntimeRole::Maintainer).await?;

    let html_state = build_html_state(&services).await?;
    let api_state = build_api_state(&services);

    let app = minne_routes(&api_state, &html_state).with_state(AppState {
        api_state,
        html_state,
    });

    info!(
        "Starting server listening on 0.0.0.0:{}",
        services.config.http_port
    );
    let serve_address = format!("0.0.0.0:{}", services.config.http_port);
    let listener = tokio::net::TcpListener::bind(serve_address).await?;

    let worker_db = Arc::clone(&services.db);
    let worker_openai = Arc::clone(&services.openai_client);
    let worker_embedding = Arc::clone(&services.embedding_provider);
    let worker_config = services.config.clone();
    let worker_reranker = services.reranker_pool.clone();
    let worker_storage = services.storage.clone();

    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let worker = tokio::spawn(async move {
        info!("Starting worker process");

        let ingestion_pipeline = Arc::new(IngestionPipeline::new(
            Arc::clone(&worker_db),
            worker_openai,
            worker_config,
            worker_reranker,
            worker_storage,
            worker_embedding,
        )?);

        run_worker_loop(worker_db, ingestion_pipeline).await
    });

    tokio::select! {
        result = server => result??,
        result = worker => result??,
    }

    Ok(())
}

#[derive(Clone, FromRef)]
struct AppState {
    api_state: api_router::api_state::ApiState,
    html_state: html_router::html_state::HtmlState,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        response::Response,
        Router,
    };
    use bootstrap::{
        prepare_embedding_runtime,
        tests::init_smoke_services,
        wiring::{build_api_state, build_html_state, minne_routes},
        EmbeddingRuntimeRole,
    };
    use common::storage::types::{system_settings::SystemSettings, user::User};
    use tower::ServiceExt;

    async fn build_test_app() -> (Router, Arc<common::storage::db::SurrealDbClient>, std::path::PathBuf) {
        let (services, data_dir) = init_smoke_services()
            .await
            .expect("failed to init services");

        prepare_embedding_runtime(&services, EmbeddingRuntimeRole::Maintainer)
            .await
            .expect("failed to prepare embedding runtime");

        let html_state = build_html_state(&services)
            .await
            .expect("failed to build html state");
        let api_state = build_api_state(&services);

        let app = minne_routes(&api_state, &html_state).with_state(AppState {
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
