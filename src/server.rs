use axum::{
    routing::post,
    Router,
    extract::Json,
};
use serde::Deserialize;
use zettle_db::rabbitmq::RabbitMQ;
use std::sync::Arc;

#[derive(Deserialize)]
struct Message {
    content: String,
}

async fn publish_message(
    Json(message): Json<Message>,
    rabbitmq: axum::extract::State<Arc<RabbitMQ>>,
) -> Result<(), String> {
    rabbitmq
        .publish_message("amq.topic", "amqprs.example", message.content)
        .await
        .map_err(|e| format!("Failed to publish message: {}", e))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rabbitmq = Arc::new(RabbitMQ::new().await?);
    let queue_name = rabbitmq.declare_queue("amqprs.examples.basic").await?.0;
    rabbitmq.bind_queue(&queue_name, "amq.topic", "amqprs.example").await?;

    let app = Router::new()
        .route("/publish", post(publish_message))
        .with_state(rabbitmq);

    println!("Server running on http://localhost:3000");
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

