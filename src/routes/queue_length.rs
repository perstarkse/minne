use axum::{http::StatusCode, response::{IntoResponse, Response}};
use tracing::{error, info};

use crate::rabbitmq::{consumer::RabbitMQConsumer, RabbitMQConfig};

pub async fn queue_length_handler() -> Response {
    info!("Getting queue length");
    
    // Set up RabbitMQ config
    let config = RabbitMQConfig {
        amqp_addr: "amqp://localhost".to_string(),
        exchange: "my_exchange".to_string(),
        queue: "my_queue".to_string(),
        routing_key: "my_key".to_string(),
    };

    // Create a new consumer
    match RabbitMQConsumer::new(&config).await {
        Ok(consumer) => {
            info!("Consumer connected to RabbitMQ");

            // Get the queue length
            let queue_length = consumer.queue.message_count();

            info!("Queue length: {}", queue_length);

            // Return the queue length with a 200 OK status
            (StatusCode::OK, queue_length.to_string()).into_response()
        },
        Err(e) => {
            error!("Failed to create consumer: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to connect to RabbitMQ".to_string()).into_response()
        }
    }
}

