#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Response,
    Router,
};
use common::{
    storage::{
        db::SurrealDbClient,
        store::StorageManager,
        types::user::User,
    },
    utils::{
        config::{AppConfig, StorageKind},
        embedding::EmbeddingProvider,
    },
};
use html_router::{
    html_routes,
    html_state::{HtmlState, StateResources},
};
use tower::ServiceExt;

async fn build_test_app() -> (Router, Arc<SurrealDbClient>) {
    let namespace = "html_router_test";
    let database = &uuid::Uuid::new_v4().to_string();
    let db = Arc::new(
        SurrealDbClient::memory(namespace, database)
            .await
            .expect("in-memory db"),
    );
    db.apply_migrations()
        .await
        .expect("migrations should apply");

    let session_store = Arc::new(
        db.create_session_store()
            .await
            .expect("session store"),
    );

    let config = AppConfig {
        storage: StorageKind::Memory,
        ..Default::default()
    };

    let storage = StorageManager::new(&config)
        .await
        .expect("storage manager");

    let embedding_provider = Arc::new(
        EmbeddingProvider::new_hashed(8).expect("embedding provider"),
    );

    let state = HtmlState::new_with_resources(StateResources {
        db: Arc::clone(&db),
        openai_client: Arc::new(async_openai::Client::new()),
        session_store,
        storage,
        config,
        reranker_pool: None,
        embedding_provider,
        template_engine: None,
    });

    let router = html_routes(&state).with_state(state);
    (router, db)
}

fn redirect_location(response: &Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .or_else(|| response.headers().get("HX-Redirect"))
        .expect("redirect response should include Location or HX-Redirect")
        .to_str()
        .expect("redirect header must be utf-8")
        .to_string()
}

fn session_cookie(response: &Response) -> String {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .expect("set-cookie must be utf-8")
                .split(';')
                .next()
                .expect("cookie key=value")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("; ")
}

async fn response_body(response: Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    String::from_utf8(body.to_vec()).expect("html body")
}

async fn sign_in(app: &Router, email: &str, password: &str) -> String {
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

    assert!(
        response.status().is_redirection() || response.status() == StatusCode::OK,
        "signin should redirect or return ok"
    );
    session_cookie(&response)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_route_redirects_unauthenticated_users() {
    let (app, _db) = build_test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("dashboard request"),
        )
        .await
        .expect("dashboard response");

    assert!(
        response.status().is_redirection() || response.status() == StatusCode::OK,
        "unauthenticated access should redirect via template middleware"
    );
    assert_eq!(redirect_location(&response), "/signin");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_user_receives_rendered_dashboard() {
    let (app, db) = build_test_app().await;

    User::create_new(
        "router_test@example.com".to_string(),
        "test_password".to_string(),
        &db,
        "UTC".to_string(),
        "system".to_string(),
    )
    .await
    .expect("test user");

    let cookie = sign_in(&app, "router_test@example.com", "test_password").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("authenticated dashboard request"),
        )
        .await
        .expect("authenticated dashboard response");

    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let html = String::from_utf8(body.to_vec()).expect("html body");
    assert!(
        html.contains("dashboard") || html.contains("Dashboard"),
        "dashboard template should render html"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signin_form_is_public() {
    let (app, _db) = build_test_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/signin")
                .body(Body::empty())
                .expect("signin form request"),
        )
        .await
        .expect("signin form response");

    assert_eq!(response.status(), StatusCode::OK);
    let html = response_body(response).await;
    assert!(
        html.contains("signin") || html.contains("Sign in") || html.contains("email"),
        "signin page should render a form"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signin_rejects_invalid_credentials() {
    let (app, db) = build_test_app().await;

    User::create_new(
        "signin_test@example.com".to_string(),
        "correct_password".to_string(),
        &db,
        "UTC".to_string(),
        "system".to_string(),
    )
    .await
    .expect("test user");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/signin")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=signin_test@example.com&password=wrong_password",
                ))
                .expect("invalid signin request"),
        )
        .await
        .expect("invalid signin response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let html = response_body(response).await;
    assert!(
        html.contains("Incorrect email or password"),
        "signin failure should render a safe error message"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_route_redirects_non_admin_user() {
    let (app, db) = build_test_app().await;

    User::create_new(
        "admin_user@example.com".to_string(),
        "admin_password".to_string(),
        &db,
        "UTC".to_string(),
        "system".to_string(),
    )
    .await
    .expect("admin user");

    User::create_new(
        "member_user@example.com".to_string(),
        "member_password".to_string(),
        &db,
        "UTC".to_string(),
        "system".to_string(),
    )
    .await
    .expect("member user");

    let member_cookie = sign_in(&app, "member_user@example.com", "member_password").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, member_cookie)
                .body(Body::empty())
                .expect("non-admin admin request"),
        )
        .await
        .expect("non-admin admin response");

    assert!(
        response.status().is_redirection() || response.status() == StatusCode::OK,
        "non-admin should be redirected away from admin"
    );
    assert_eq!(redirect_location(&response), "/");

    let admin_cookie = sign_in(&app, "admin_user@example.com", "admin_password").await;
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
}
