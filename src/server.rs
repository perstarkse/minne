use axum::{
         extract::DefaultBodyLimit, routing::{delete, get, post, put}, Extension, Router
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{rabbitmq::{publisher::RabbitMQProducer, RabbitMQConfig}, routes::{file::{delete_file_handler, get_file_handler, update_file_handler, upload_handler}, ingress::ingress_handler, queue_length::queue_length_handler}, surrealdb::SurrealDbClient};
use std::sync::Arc;

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
    
    // Set up producer
    let producer = Arc::new(RabbitMQProducer::new(&config).await?);

    // Set up database client
    let db_client = Arc::new(SurrealDbClient::new().await?);

    
    // Create Axum router
    let app = Router::new()
        .route("/ingress", post(ingress_handler))
        .route("/message_count", get(queue_length_handler))
        .layer(Extension(producer))
        .route("/file", post(upload_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
        .route("/file/:uuid", get(get_file_handler)) 
        .route("/file/:uuid", put(update_file_handler)) 
        .route("/file/:uuid", delete(delete_file_handler))
        .layer(Extension(db_client));
     
    tracing::info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}


