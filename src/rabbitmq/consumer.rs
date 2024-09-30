use lapin::{
    message::Delivery, options::*, types::FieldTable, Channel, Consumer, Queue 
};
use futures_lite::stream::StreamExt;

use crate::models::{ingress_content::IngressContentError, ingress_object::IngressObject, text_content::TextContent};

use super::{RabbitMQCommon, RabbitMQConfig, RabbitMQError};
use tracing::{info, error};

/// Struct to consume messages from RabbitMQ.
pub struct RabbitMQConsumer {
    common: RabbitMQCommon,
    pub queue: Queue,
    consumer: Consumer,
}

impl RabbitMQConsumer {
    //// Creates a new 'RabbitMQConsumer' instance which sets up a rabbitmq client,
    //// declares a exchange if needed, declares and binds a queue and initializes the consumer
    ////
    //// # Arguments
    ////
    //// * 'config' - RabbitMQConfig
    ////
    //// # Returns
    ////
    //// * 'Result<Self, RabbitMQError>' - The created client or an error.
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let common = RabbitMQCommon::new(config).await?;
        
        // Passively declare the exchange (it should already exist)
        common.declare_exchange(config, true).await?;
        
        // Declare queue and bind it to the channel
        let queue = Self::declare_queue(&common.channel, config).await?;
        Self::bind_queue(&common.channel, &config.exchange, &queue, config).await?;
        
        // Initialize the consumer
        let consumer = Self::initialize_consumer(&common.channel, &config).await?;

        Ok(Self { common, queue, consumer })
    }

    async fn initialize_consumer(channel: &Channel, config: &RabbitMQConfig) -> Result<Consumer, RabbitMQError> {
        channel
            .basic_consume(
                &config.queue,
                "consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await.map_err(|e| RabbitMQError::InitializeConsumerError(e.to_string()))
    }

    async fn declare_queue(channel: &Channel, config: &RabbitMQConfig) -> Result<Queue, RabbitMQError> {
        channel
            .queue_declare(
                &config.queue,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| RabbitMQError::QueueError(e.to_string()))
    }

    async fn bind_queue(channel: &Channel, exchange: &str, queue: &Queue, config: &RabbitMQConfig) -> Result<(), RabbitMQError> {
        channel
            .queue_bind(
                queue.name().as_str(),
                exchange,
                &config.routing_key,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| RabbitMQError::QueueError(e.to_string()))
    }

    /// Consumes a message and returns the deserialized IngressContent along with the Delivery
    pub async fn consume(&self) -> Result<(IngressObject, Delivery), RabbitMQError> {
        // Receive the next message
        let delivery = self.consumer.clone().next().await
            .ok_or_else(|| RabbitMQError::ConsumeError("No message received".to_string()))?
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        // Deserialize the message payload into IngressContent
        let ingress: IngressObject = serde_json::from_slice(&delivery.data)
            .map_err(|e| RabbitMQError::ConsumeError(format!("Deserialization Error: {}", e)))?;

        Ok((ingress, delivery))
    }

    /// Acknowledges the message after processing
    pub async fn ack_delivery(&self, delivery: Delivery) -> Result<(), RabbitMQError> {
        self.common.channel
            .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
            .await
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        Ok(())
    }
    pub async fn process_messages(&self) -> Result<(), RabbitMQError> {
        loop {
            match self.consume().await {
                Ok((ingress, delivery)) => {
                    info!("Received IngressObject: {:?}", ingress);
                    let text_content = ingress.to_text_content().await.unwrap();
                    text_content.process().await.unwrap();
                    
                    self.ack_delivery(delivery).await?;
                    // Process the IngressContent
                    // match self.handle_ingress_content(&ingress).await {
                    //     Ok(_) => {
                    //         info!("Successfully handled IngressContent");
                    //         // Acknowledge the message
                    //         if let Err(e) = self.ack_delivery(delivery).await {
                    //             error!("Failed to acknowledge message: {:?}", e);
                    //         }
                    //     },
                    //     Err(e) => {
                    //         error!("Failed to handle IngressContent: {:?}", e);
                    //         // For now, we'll acknowledge to remove it from the queue. Change to nack?
                    //         if let Err(ack_err) = self.ack_delivery(delivery).await {
                    //             error!("Failed to acknowledge message after handling error: {:?}", ack_err);
                    //         }
                    //     }
                    // }
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

    pub async fn handle_ingress_content(&self, ingress: &IngressObject) -> Result<(), IngressContentError> {
        info!("Processing IngressContent: {:?}", ingress);

        unimplemented!()
    }
}
