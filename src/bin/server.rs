use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{get, post},
    Router,
};
use axum_session::{SessionConfig, SessionLayer, SessionStore};
use axum_session_auth::{AuthConfig, AuthSessionLayer};
use axum_session_surreal::SessionSurrealPool;
use minijinja::{path_loader, Environment};
use minijinja_autoreload::AutoReloader;
use std::{path::PathBuf, sync::Arc};
use surrealdb::{engine::any::Any, Surreal};
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{
    rabbitmq::{consumer::RabbitMQConsumer, publisher::RabbitMQProducer, RabbitMQConfig},
    server::{
        middleware_api_auth::api_auth,
        routes::{
            api::{
                file::upload_handler, ingress::ingress_handler, query::query_handler,
                queue_length::queue_length_handler,
            },
            html::{
                index::index_handler,
                search_result::search_result_handler,
                signout::sign_out_user,
                signup::{process_signup_and_show_verification, show_signup_form},
            },
        },
        AppState,
    },
    storage::{db::SurrealDbClient, types::user::User},
    utils::{config::get_config, mailer::Mailer},
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;

    info!("{:?}", config);

    // Set up RabbitMQ
    let rabbitmq_config = RabbitMQConfig {
        amqp_addr: config.rabbitmq_address,
        exchange: config.rabbitmq_exchange,
        queue: config.rabbitmq_queue,
        routing_key: config.rabbitmq_routing_key,
    };

    let reloader = AutoReloader::new(move |notifier| {
        let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
        let mut env = Environment::new();
        env.set_loader(path_loader(&template_path));

        notifier.set_fast_reload(true);
        notifier.watch_path(&template_path, true);
        Ok(env)
    });

    let app_state = AppState {
        rabbitmq_producer: Arc::new(RabbitMQProducer::new(&rabbitmq_config).await?),
        rabbitmq_consumer: Arc::new(RabbitMQConsumer::new(&rabbitmq_config, false).await?),
        surreal_db_client: Arc::new(
            SurrealDbClient::new(
                &config.surrealdb_address,
                &config.surrealdb_username,
                &config.surrealdb_password,
                &config.surrealdb_namespace,
                &config.surrealdb_database,
            )
            .await?,
        ),
        openai_client: Arc::new(async_openai::Client::new()),
        templates: Arc::new(reloader),
        mailer: Arc::new(Mailer::new(
            config.smtp_username,
            config.smtp_relayer,
            config.smtp_password,
        )?),
    };

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

    app_state.surreal_db_client.build_indexes().await?;

    // Create Axum router
    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&app_state))
        .nest(
            "/",
            html_routes(
                session_store,
                auth_config,
                app_state.surreal_db_client.client.clone(),
            ),
        )
        .with_state(app_state);

    info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Router for API functionality, version 1
fn api_routes_v1(app_state: &AppState) -> Router<AppState> {
    Router::new()
        // Ingress routes
        .route("/ingress", post(ingress_handler))
        .route("/message_count", get(queue_length_handler))
        // File routes
        .route("/file", post(upload_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        // Query routes
        .route("/query", post(query_handler))
        .route_layer(from_fn_with_state(app_state.clone(), api_auth))
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
        .route("/signout", get(sign_out_user))
        .route(
            "/signup",
            get(show_signup_form).post(process_signup_and_show_verification),
        )
        .nest_service("/assets", ServeDir::new("assets/"))
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
