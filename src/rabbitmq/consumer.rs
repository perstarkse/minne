use futures::StreamExt;
use lapin::{message::Delivery, options::*, types::FieldTable, Channel, Consumer, Queue};

use crate::{
    error::IngressConsumerError,
    ingress::{content_processor::ContentProcessor, types::ingress_object::IngressObject},
};

use super::{RabbitMQCommon, RabbitMQCommonTrait, RabbitMQConfig, RabbitMQError};
use tracing::{error, info};

/// Struct to consume messages from RabbitMQ.
pub struct RabbitMQConsumer {
    common: RabbitMQCommon,
    pub queue: Queue,
    consumer: Consumer,
}

impl RabbitMQConsumer {
    /// Creates a new 'RabbitMQConsumer' instance which sets up a rabbitmq client,
    /// declares a exchange if needed, declares and binds a queue and initializes the consumer
    ///
    /// # Arguments
    /// * `config` - A initialized RabbitMQConfig containing required configurations
    ///
    /// # Returns
    /// * `Result<Self, RabbitMQError>` - The created client or an error.
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let common = RabbitMQCommon::new(config).await?;

        // Passively declare the exchange (it should already exist)
        common.declare_exchange(config, true).await?;

        // Declare queue and bind it to the channel
        let queue = Self::declare_queue(&common.channel, config).await?;
        Self::bind_queue(&common.channel, &config.exchange, &queue, config).await?;

        // Initialize the consumer
        let consumer = Self::initialize_consumer(&common.channel, config).await?;

        Ok(Self {
            common,
            queue,
            consumer,
        })
    }

    /// Sets up the consumer based on the channel and `RabbitMQConfig`.
    ///
    /// # Arguments
    /// * `channel` - Lapin Channel.
    /// * `config` - A initialized RabbitMQConfig containing required information
    ///
    /// # Returns
    /// * `Result<Consumer, RabbitMQError>` - The initialized consumer or error
    async fn initialize_consumer(
        channel: &Channel,
        config: &RabbitMQConfig,
    ) -> Result<Consumer, RabbitMQError> {
        channel
            .basic_consume(
                &config.queue,
                "consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| RabbitMQError::InitializeConsumerError(e.to_string()))
    }
    /// Declares the queue based on the channel and `RabbitMQConfig`.
    /// # Arguments
    /// * `channel` - Lapin Channel.
    /// * `config` - A initialized RabbitMQConfig containing required information
    ///
    /// # Returns
    /// * `Result<Queue, RabbitMQError>` - The initialized queue or error
    async fn declare_queue(
        channel: &Channel,
        config: &RabbitMQConfig,
    ) -> Result<Queue, RabbitMQError> {
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

    /// Binds the queue based on the channel, declared exchange, queue and `RabbitMQConfig`.
    /// # Arguments
    /// * `channel` - Lapin Channel.
    /// * `exchange` - String value of the exchange name
    /// * `queue` - Lapin queue thats declared
    /// * `config` - A initialized RabbitMQConfig containing required information
    ///
    /// # Returns
    /// * `Result<(), RabbitMQError>` - Ok or error
    async fn bind_queue(
        channel: &Channel,
        exchange: &str,
        queue: &Queue,
        config: &RabbitMQConfig,
    ) -> Result<(), RabbitMQError> {
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

    /// Consumes a message and returns the message along with delivery details.
    ///
    /// # Arguments
    /// * `&self` - A reference to self
    ///
    /// # Returns
    /// `IngressObject` - The object containing content and metadata.
    /// `Delivery` - A delivery reciept, required to ack or nack the delivery.
    pub async fn consume(&self) -> Result<(IngressObject, Delivery), RabbitMQError> {
        // Receive the next message
        let delivery = self
            .consumer
            .clone()
            .next()
            .await
            .ok_or_else(|| RabbitMQError::ConsumeError("No message received".to_string()))?
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        // Deserialize the message payload into IngressContent
        let ingress: IngressObject = serde_json::from_slice(&delivery.data)
            .map_err(|e| RabbitMQError::ConsumeError(format!("Deserialization Error: {}", e)))?;

        Ok((ingress, delivery))
    }

    /// Acknowledges the message after processing
    ///
    /// # Arguments
    /// * `self` - Reference to self
    /// * `delivery` - Delivery reciept
    ///
    /// # Returns
    /// * `Result<(), RabbitMQError>` - Ok or error
    pub async fn ack_delivery(&self, delivery: Delivery) -> Result<(), RabbitMQError> {
        self.common
            .channel
            .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
            .await
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        Ok(())
    }
    /// Function to continually consume messages as they come in
    pub async fn process_messages(&self) -> Result<(), IngressConsumerError> {
        loop {
            match self.consume().await {
                Ok((ingress, delivery)) => {
                    info!("Received IngressObject: {:?}", ingress);
                    // Get the TextContent
                    let text_content = ingress.to_text_content().await?;

                    // Initialize ContentProcessor which handles LLM analysis and storage
                    let content_processor = ContentProcessor::new().await?;

                    // Begin processing of TextContent
                    content_processor.process(&text_content).await?;

                    // Remove from queue
                    self.ack_delivery(delivery).await?;
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
}
