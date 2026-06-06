#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Response,
    Router,
};
use common::{
    storage::{db::SurrealDbClient, store::StorageManager, types::user::User},
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

    let session_store = Arc::new(db.create_session_store().await.expect("session store"));

    let config = AppConfig {
        storage: StorageKind::Memory,
        ..Default::default()
    };

    let storage = StorageManager::new(&config).await.expect("storage manager");

    let embedding_provider =
        Arc::new(EmbeddingProvider::new_hashed(8).expect("embedding provider"));

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

/// Shared insta settings for HTML snapshots in this module.
///
/// The in-memory DB is recreated per test, so ids in markup would otherwise churn.
/// Filters normalize those values; see `snapshot_*` tests below.
fn snapshot_settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.add_filter(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        "[uuid]",
    );
    settings.add_filter(r"[a-z_]+:[0-9a-z]{12,}", "[record-id]");
    settings
}

const AUTHENTICATED_MAIN_OPEN: &str = r#"<main class="flex flex-col flex-1 overflow-y-auto">"#;

/// Inner HTML of the scrollable page column from `body_base.html` (`{% block main %}`).
///
/// Omits head, navbar shell, sidebar, and modal mount points so per-route snapshots
/// do not duplicate layout chrome (see `snapshot_authenticated_shell`).
fn extract_authenticated_main(html: &str) -> &str {
    let start = html
        .find(AUTHENTICATED_MAIN_OPEN)
        .expect("authenticated page main column")
        .saturating_add(AUTHENTICATED_MAIN_OPEN.len());
    let rest = &html[start..];
    let end = rest
        .find("</main>")
        .expect("authenticated page main column close");
    &rest[..end]
}

async fn get_html(app: &Router, uri: &str, cookie: Option<&str>) -> String {
    let mut builder = Request::builder().uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    response_body(response).await
}

/// Fixed credentials for authenticated snapshot routes (dashboard, search, etc.).
async fn seeded_cookie(app: &Router, db: &SurrealDbClient) -> String {
    User::create_new(
        "snapshot_user@example.com".to_string(),
        "snapshot_password".to_string(),
        db,
        "UTC".to_string(),
        "system".to_string(),
    )
    .await
    .expect("snapshot user");
    sign_in(app, "snapshot_user@example.com", "snapshot_password").await
}

/// Parses a scratchpad id from the list page HTML after `POST /scratchpad`.
async fn create_scratchpad_and_get_id(app: &Router, cookie: &str, title: &str) -> String {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/scratchpad")
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("title={title}")))
                .expect("create request"),
        )
        .await
        .expect("create response");

    let list = get_html(app, "/scratchpad", Some(cookie)).await;
    let marker = "/scratchpad/";
    let start = list
        .find(marker)
        .expect("scratchpad link present")
        .saturating_add(marker.len());
    let end = start.saturating_add(list[start..].find('/').expect("id terminator"));
    list[start..end].to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scratchpad_editor_modal_does_not_nest_forms() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let id = create_scratchpad_and_get_id(&app, &cookie, "IngestPad").await;

    let modal = get_html(&app, &format!("/scratchpad/{id}/modal"), Some(&cookie)).await;

    // Scratchpad editor opts out of #modal_form (see editor_modal.html); nested
    // <form> elements are invalid HTML and browsers drop the inner forms.
    assert!(
        !modal.contains(r#"id="modal_form""#),
        "editor modal should not wrap content in #modal_form"
    );
    assert!(
        modal.contains(&format!("/scratchpad/{id}/ingest")),
        "ingest form action should be present"
    );
    assert!(
        modal.contains(r#"id="ingest-form""#),
        "ingest form should be a real, addressable form"
    );

    // Ingest targets #main_section, so the response must be a partial, not a full page.
    app.clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/scratchpad/{id}/auto-save"))
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("content=Some+content+to+ingest"))
                .expect("save request"),
        )
        .await
        .expect("save response");

    let ingest = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/scratchpad/{id}/ingest"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("ingest request"),
        )
        .await
        .expect("ingest response");
    assert_eq!(ingest.status(), StatusCode::OK);
    let body = response_body(ingest).await;
    assert!(
        !body.trim_start().starts_with("<!DOCTYPE") && body.contains(r#"id="main_section""#),
        "ingest should return only the main section partial"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scratchpad_archive_returns_main_partial_only() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let id = create_scratchpad_and_get_id(&app, &cookie, "RegressionPad").await;

    let archive = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/scratchpad/{id}/archive"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("archive request"),
        )
        .await
        .expect("archive response");
    assert_eq!(archive.status(), StatusCode::OK);

    let body = response_body(archive).await;
    // Archive uses hx-target="#main_section" — same partial contract as ingest.
    assert!(
        !body.trim_start().starts_with("<!DOCTYPE"),
        "archive should return a partial, not a full document"
    );
    assert!(
        !body.contains("drawer-side"),
        "archive partial should not include the sidebar"
    );
    assert!(
        body.contains(r#"id="main_section""#),
        "archive partial should be the main section"
    );
}

// HTML regression snapshots (insta). Authenticated layout: one full-document shell
// plus per-route main-column slices via `extract_authenticated_main`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_signin_page() {
    let (app, _db) = build_test_app().await;
    let body = get_html(&app, "/signin", None).await;
    snapshot_settings().bind(|| insta::assert_snapshot!("signin_page", body));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_signup_page() {
    let (app, _db) = build_test_app().await;
    let body = get_html(&app, "/signup", None).await;
    snapshot_settings().bind(|| insta::assert_snapshot!("signup_page", body));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_authenticated_shell() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let body = get_html(&app, "/", Some(&cookie)).await;
    snapshot_settings().bind(|| insta::assert_snapshot!("authenticated_shell", body));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_dashboard_main() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let body = get_html(&app, "/", Some(&cookie)).await;
    let main = extract_authenticated_main(&body);
    snapshot_settings().bind(|| insta::assert_snapshot!("dashboard_main", main));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_search_main() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let body = get_html(&app, "/search", Some(&cookie)).await;
    let main = extract_authenticated_main(&body);
    snapshot_settings().bind(|| insta::assert_snapshot!("search_main", main));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_not_found_main() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let body = get_html(&app, "/file/does-not-exist", Some(&cookie)).await;
    let main = extract_authenticated_main(&body);
    snapshot_settings().bind(|| insta::assert_snapshot!("not_found_main", main));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_new_entity_modal() {
    let (app, db) = build_test_app().await;
    let cookie = seeded_cookie(&app, &db).await;
    let body = get_html(&app, "/knowledge-entity/new", Some(&cookie)).await;
    snapshot_settings().bind(|| insta::assert_snapshot!("new_entity_modal", body));
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
