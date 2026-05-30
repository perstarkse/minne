#![allow(clippy::expect_used)]

use std::sync::Arc;

use api_router::{api_routes_v1, api_state::ApiState};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use common::{
    storage::{
        db::SurrealDbClient,
        store::StorageManager,
        types::user::User,
    },
    utils::config::{AppConfig, StorageKind},
};
use tower::ServiceExt;

async fn build_test_app() -> (Router, Arc<SurrealDbClient>) {
    let namespace = "api_router_test";
    let database = uuid::Uuid::new_v4().to_string();
    let db = Arc::new(
        SurrealDbClient::memory(namespace, &database)
            .await
            .expect("in-memory db"),
    );
    db.apply_migrations()
        .await
        .expect("migrations should apply");

    let config = AppConfig {
        storage: StorageKind::Memory,
        ..Default::default()
    };
    let storage = StorageManager::new(&config)
        .await
        .expect("storage manager");

    let state = ApiState {
        db: Arc::clone(&db),
        config,
        storage,
    };

    let router = api_routes_v1(&state).with_state(state);

    (router, db)
}

async fn response_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    String::from_utf8(body.to_vec()).expect("utf-8 body")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_probe_is_public() {
    let (app, _db) = build_test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/live")
                .body(Body::empty())
                .expect("live request"),
        )
        .await
        .expect("live response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response_body(response).await.contains("\"status\":\"ok\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_probe_is_public_and_reports_db_ok() {
    let (app, _db) = build_test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("ready request"),
        )
        .await
        .expect("ready response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_body(response).await;
    assert!(body.contains("\"checks\":{\"db\":\"ok\"}") || body.contains("\"db\":\"ok\""));
    assert!(!body.contains("reason"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_route_requires_api_key() {
    let (app, _db) = build_test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/categories")
                .body(Body::empty())
                .expect("categories request"),
        )
        .await
        .expect("categories response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_route_rejects_invalid_api_key() {
    let (app, _db) = build_test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/categories")
                .header("X-API-Key", "sk_invalid")
                .body(Body::empty())
                .expect("categories request"),
        )
        .await
        .expect("categories response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_user_can_list_categories() {
    let (app, db) = build_test_app().await;

    let user = User::create_new(
        "api_router_test@example.com".to_string(),
        "test_password".to_string(),
        &db,
        "UTC".to_string(),
        "system".to_string(),
    )
    .await
    .expect("test user");

    let api_key = User::set_api_key(&user.id, &db)
        .await
        .expect("api key");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/categories")
                .header("X-API-Key", api_key)
                .body(Body::empty())
                .expect("categories request"),
        )
        .await
        .expect("categories response");

    assert_eq!(response.status(), StatusCode::OK);
}
