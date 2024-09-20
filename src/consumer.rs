use lapin::{
    options::*, types::FieldTable, Connection, ConnectionProperties, Consumer, Result,
};
use tracing::{info, error};
use futures_lite::stream::StreamExt;

pub struct RabbitMQConsumer {
    pub connection: Connection,
    pub    consumer: Consumer,
}

impl RabbitMQConsumer {
    pub async fn new(addr: &str, queue: &str) -> Result<Self> {
        let connection = Connection::connect(addr, ConnectionProperties::default()).await?;
        let channel = connection.create_channel().await?;

        // Declare the queue (in case it doesn't exist)
        channel
            .queue_declare(
                queue,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        let consumer = channel
            .basic_consume(
                queue,
                "consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;

        Ok(Self { connection, consumer })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Consumer started - waiting for messages");
        while let Some(delivery) = self.consumer.next().await {
            match delivery {
                Ok(delivery) => {
                    let message = std::str::from_utf8(&delivery.data).unwrap_or("Invalid UTF-8");
                    info!("Received message: {}", message);
                    
                    // Process the message here
                    // For example, you could deserialize it and perform some action
                    
                    delivery.ack(BasicAckOptions::default()).await?;
                },
                Err(e) => error!("Failed to consume message: {:?}", e),
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up tracing
    tracing_subscriber::fmt::init();

    let addr = "amqp://guest:guest@localhost:5672";
    let queue = "hello";

    let mut consumer = RabbitMQConsumer::new(addr, queue).await?;
    
    info!("Starting consumer");
    consumer.run().await?;

    Ok(())
}
