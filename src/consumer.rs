use tokio;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::rabbitmq::{consumer::RabbitMQConsumer, RabbitMQConfig };

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    info!("Starting RabbitMQ consumer");
    
    // Set up RabbitMQ config
        let config = RabbitMQConfig {
        amqp_addr: "amqp://localhost".to_string(),
        exchange: "my_exchange".to_string(),
        queue: "my_queue".to_string(),
        routing_key: "my_key".to_string(),
    };


    // Create a RabbitMQ consumer
    let consumer = RabbitMQConsumer::new(&config).await?;

    info!("Consumer connected to RabbitMQ");

    // Start consuming messages
    consumer.process_messages().await?;

    Ok(())
}
