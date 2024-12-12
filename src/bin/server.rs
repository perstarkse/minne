use axum::{
    extract::DefaultBodyLimit,
    http::Method,
    routing::{get, post},
    Router,
};
use axum_session::{SessionConfig, SessionLayer, SessionStore};
use axum_session_auth::{Auth, AuthConfig, AuthSession, AuthSessionLayer, Rights};
use axum_session_surreal::SessionSurrealPool;
use std::sync::Arc;
use surrealdb::{engine::any::Any, Surreal};
use tera::Tera;
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{
    rabbitmq::{consumer::RabbitMQConsumer, publisher::RabbitMQProducer, RabbitMQConfig},
    server::{
        routes::{
            auth::{show_signup_form, signup_handler},
            file::upload_handler,
            index::index_handler,
            ingress::ingress_handler,
            query::query_handler,
            queue_length::queue_length_handler,
            search_result::search_result_handler,
        },
        AppState,
    },
    storage::{db::SurrealDbClient, types::user::User},
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    // Set up RabbitMQ
    let config = RabbitMQConfig {
        amqp_addr: "amqp://localhost".to_string(),
        exchange: "my_exchange".to_string(),
        queue: "my_queue".to_string(),
        routing_key: "my_key".to_string(),
    };

    let app_state = AppState {
        rabbitmq_producer: Arc::new(RabbitMQProducer::new(&config).await?),
        rabbitmq_consumer: Arc::new(RabbitMQConsumer::new(&config, false).await?),
        surreal_db_client: Arc::new(SurrealDbClient::new().await?),
        tera: Arc::new(Tera::new("src/server/templates/**/*.html").unwrap()),
        openai_client: Arc::new(async_openai::Client::new()),
    };
    // app_state.surreal_db_client.query("DELETE user").await?;

    // setup_auth(&app_state.surreal_db_client).await?;

    let session_config = SessionConfig::default()
        .with_table_name("test_session_table")
        .with_secure(false);
    let auth_config = AuthConfig::<String>::default();

    let session_store: SessionStore<SessionSurrealPool<Any>> = SessionStore::new(
        Some(app_state.surreal_db_client.client.clone().into()),
        session_config,
    )
    .await?;

    // Create Axum router
    let app = Router::new()
        .nest("/api/v1", api_routes_v1())
        .nest(
            "/",
            html_routes(
                session_store,
                auth_config,
                app_state.surreal_db_client.client.clone(),
            ),
        )
        .with_state(app_state);

    tracing::info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Router for API functionality, version 1
fn api_routes_v1() -> Router<AppState> {
    Router::new()
        // Ingress routes
        .route("/ingress", post(ingress_handler))
        .route("/message_count", get(queue_length_handler))
        // File routes
        .route("/file", post(upload_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        // Query routes
        .route("/query", post(query_handler))
}

/// Router for HTML endpoints
///
fn html_routes(
    session_store: SessionStore<SessionSurrealPool<Any>>,
    auth_config: AuthConfig<String>,
    db_client: Surreal<Any>,
) -> Router<AppState> {
    Router::new()
        .route("/", get(index_handler))
        .route("/search", get(search_result_handler))
        .route("/signup", get(show_signup_form).post(signup_handler))
        .nest_service("/assets", ServeDir::new("src/server/assets"))
        .layer(
            AuthSessionLayer::<User, String, SessionSurrealPool<Any>, Surreal<Any>>::new(Some(
                db_client,
            ))
            .with_config(auth_config),
        )
        .layer(SessionLayer::new(session_store))
}

// async fn setup_auth(db: &SurrealDbClient) -> Result<(), Box<dyn std::error::Error>> {
//     db.query(
//         "DEFINE TABLE user SCHEMALESS;
//         DEFINE INDEX unique_name ON TABLE user FIELDS email UNIQUE;
//         DEFINE ACCESS account ON DATABASE TYPE RECORD
//         SIGNUP ( CREATE user SET email = $email, password = crypto::argon2::generate($password), anonymous = false, user_id = $user_id)
//         SIGNIN ( SELECT * FROM user WHERE email = $email AND crypto::argon2::compare(password, $password) );",
//     )
//     .await?;
//     Ok(())
// }
