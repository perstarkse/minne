use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::{
    ingress::content_processor::ContentProcessor,
    rabbitmq::{consumer::RabbitMQConsumer, RabbitMQConfig, RabbitMQError},
    utils::config::get_config,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    info!("Starting RabbitMQ consumer");

    let config = get_config()?;

    // Set up RabbitMQ config
    let rabbitmq_config = RabbitMQConfig {
        amqp_addr: config.rabbitmq_address.clone(),
        exchange: config.rabbitmq_exchange.clone(),
        queue: config.rabbitmq_queue.clone(),
        routing_key: config.rabbitmq_routing_key.clone(),
    };

    // Create a RabbitMQ consumer
    let consumer = RabbitMQConsumer::new(&rabbitmq_config, true).await?;

    // Start consuming messages
    loop {
        match consumer.consume().await {
            Ok((ingress, delivery)) => {
                info!("Received IngressObject: {:?}", ingress);
                // Get the TextContent
                let text_content = ingress.to_text_content().await?;

                // Initialize ContentProcessor which handles LLM analysis and storage
                let content_processor = ContentProcessor::new(&config).await?;

                // Begin processing of TextContent
                content_processor.process(&text_content).await?;

                // Remove from queue
                consumer.ack_delivery(delivery).await?;
            }
            Err(RabbitMQError::ConsumeError(e)) => {
                error!("Error consuming message: {}", e);
                // Optionally add a delay before trying again
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
            Err(e) => {
                error!("Unexpected error: {}", e);
                break;
            }
        }
    }

    Ok(())
}
