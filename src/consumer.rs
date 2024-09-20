// use lapin::{
//     options::*, types::FieldTable, Connection, ConnectionProperties, Consumer, Result,
// };
// use tracing::{info, error};
// use futures_lite::stream::StreamExt;

// pub struct RabbitMQConsumer {
//     pub connection: Connection,
//     pub    consumer: Consumer,
// }

// impl RabbitMQConsumer {
//     pub async fn new(addr: &str, queue: &str) -> Result<Self> {
//         let connection = Connection::connect(addr, ConnectionProperties::default()).await?;
//         let channel = connection.create_channel().await?;

//         // Declare the queue (in case it doesn't exist)
//         channel
//             .queue_declare(
//                 queue,
//                 QueueDeclareOptions::default(),
//                 FieldTable::default(),
//             )
//             .await?;

//         let consumer = channel
//             .basic_consume(
//                 queue,
//                 "consumer",
//                 BasicConsumeOptions::default(),
//                 FieldTable::default(),
//             )
//             .await?;

//         Ok(Self { connection, consumer })
//     }

//     pub async fn run(&mut self) -> Result<()> {
//         info!("Consumer started - waiting for messages");
//         while let Some(delivery) = self.consumer.next().await {
//             match delivery {
//                 Ok(delivery) => {
//                     let message = std::str::from_utf8(&delivery.data).unwrap_or("Invalid UTF-8");
//                     info!("Received message: {}", message);

//                     // Process the message here
//                     // For example, you could deserialize it and perform some action

//                     delivery.ack(BasicAckOptions::default()).await?;
//                 },
//                 Err(e) => error!("Failed to consume message: {:?}", e),
//             }
//         }

//         Ok(())
//     }
// }

// #[tokio::main]
// async fn main() -> Result<()> {
//     // Set up tracing
//     tracing_subscriber::fmt::init();

//     let addr = "amqp://guest:guest@localhost:5672";
//     let queue = "hello";

//     let mut consumer = RabbitMQConsumer::new(addr, queue).await?;

//     info!("Starting consumer");
//     consumer.run().await?;

//     Ok(())
// }
use tokio;
use tracing::{info, error};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::rabbitmq::{consumer::RabbitMQConsumer, RabbitMQConfig, RabbitMQError};

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
    loop {
        match consumer.consume().await {
            Ok(message) => {
                info!("Received message: {}", message);
                // Process the message here
                // For example, you could insert it into a database
                // process_message(&message).await?;
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
