use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post},
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
    ingress::jobqueue::JobQueue,
    server::{
        middleware_analytics::analytics_middleware,
        middleware_api_auth::api_auth,
        routes::{
            api::{
                ingress::ingress_data,
                ingress_task::{delete_queue_task, get_queue_tasks},
                query::query_handler,
                queue_length::queue_length_handler,
            },
            html::{
                account::{delete_account, set_api_key, show_account_page, update_timezone},
                admin_panel::{show_admin_panel, toggle_registration_status},
                documentation::index::show_documentation_index,
                gdpr::{accept_gdpr, deny_gdpr},
                index::{delete_job, delete_text_content, index_handler},
                ingress_form::{hide_ingress_form, process_ingress_form, show_ingress_form},
                privacy_policy::show_privacy_policy,
                search_result::search_result_handler,
                signin::{authenticate_user, show_signin_form},
                signout::sign_out_user,
                signup::{process_signup_and_show_verification, show_signup_form},
            },
        },
        AppState,
    },
    storage::{
        db::SurrealDbClient,
        types::{analytics::Analytics, system_settings::SystemSettings, user::User},
    },
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

    let reloader = AutoReloader::new(move |notifier| {
        let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
        let mut env = Environment::new();
        env.set_loader(path_loader(&template_path));

        notifier.set_fast_reload(true);
        notifier.watch_path(&template_path, true);
        minijinja_contrib::add_to_environment(&mut env);
        Ok(env)
    });

    let surreal_db_client = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    );

    let openai_client = Arc::new(async_openai::Client::new());

    let app_state = AppState {
        surreal_db_client: surreal_db_client.clone(),
        templates: Arc::new(reloader),
        openai_client: openai_client.clone(),
        mailer: Arc::new(Mailer::new(
            config.smtp_username,
            config.smtp_relayer,
            config.smtp_password,
        )?),
        job_queue: Arc::new(JobQueue::new(surreal_db_client, openai_client)),
    };

    let session_config = SessionConfig::default()
        .with_table_name("test_session_table")
        .with_secure(true);
    let auth_config = AuthConfig::<String>::default();

    let session_store: SessionStore<SessionSurrealPool<Any>> = SessionStore::new(
        Some(app_state.surreal_db_client.client.clone().into()),
        session_config,
    )
    .await?;

    app_state.surreal_db_client.build_indexes().await?;
    setup_auth(&app_state.surreal_db_client).await?;
    Analytics::ensure_initialized(&app_state.surreal_db_client).await?;
    SystemSettings::ensure_initialized(&app_state.surreal_db_client).await?;

    // Create Axum router
    let app = Router::new()
        .nest("/api/v1", api_routes_v1(&app_state))
        .nest(
            "/",
            html_routes(
                session_store,
                auth_config,
                app_state.surreal_db_client.client.clone(),
                &app_state,
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
        .route("/ingress", post(ingress_data))
        .route("/message_count", get(queue_length_handler))
        .route("/queue", get(get_queue_tasks))
        .route("/queue/:delivery_tag", delete(delete_queue_task))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        // Query routes
        .route("/query", post(query_handler))
        .route_layer(from_fn_with_state(app_state.clone(), api_auth))
}

/// Router for HTML endpoints
fn html_routes(
    session_store: SessionStore<SessionSurrealPool<Any>>,
    auth_config: AuthConfig<String>,
    db_client: Surreal<Any>,
    app_state: &AppState,
) -> Router<AppState> {
    Router::new()
        .route("/", get(index_handler))
        .route("/gdpr/accept", post(accept_gdpr))
        .route("/gdpr/deny", post(deny_gdpr))
        .route("/search", get(search_result_handler))
        .route("/signout", get(sign_out_user))
        .route("/signin", get(show_signin_form).post(authenticate_user))
        .route(
            "/ingress-form",
            get(show_ingress_form).post(process_ingress_form),
        )
        .route("/hide-ingress-form", get(hide_ingress_form))
        .route("/text-content/:id", delete(delete_text_content))
        .route("/jobs/:job_id", delete(delete_job))
        .route("/account", get(show_account_page))
        .route("/admin", get(show_admin_panel))
        .route("/toggle-registrations", patch(toggle_registration_status))
        .route("/set-api-key", post(set_api_key))
        .route("/update-timezone", patch(update_timezone))
        .route("/delete-account", delete(delete_account))
        .route(
            "/signup",
            get(show_signup_form).post(process_signup_and_show_verification),
        )
        .route("/documentation", get(show_documentation_index))
        .route("/documentation/privacy-policy", get(show_privacy_policy))
        .nest_service("/assets", ServeDir::new("assets/"))
        .layer(from_fn_with_state(app_state.clone(), analytics_middleware))
        .layer(
            AuthSessionLayer::<User, String, SessionSurrealPool<Any>, Surreal<Any>>::new(Some(
                db_client,
            ))
            .with_config(auth_config),
        )
        .layer(SessionLayer::new(session_store))
}

async fn setup_auth(db: &SurrealDbClient) -> Result<(), Box<dyn std::error::Error>> {
    db.query(
        "DEFINE TABLE user SCHEMALESS;
        DEFINE INDEX unique_name ON TABLE user FIELDS email UNIQUE;
        DEFINE ACCESS account ON DATABASE TYPE RECORD
        SIGNUP ( CREATE user SET email = $email, password = crypto::argon2::generate($password), anonymous = false, user_id = $user_id)
        SIGNIN ( SELECT * FROM user WHERE email = $email AND crypto::argon2::compare(password, $password) );",
    )
    .await?;
    Ok(())
}
