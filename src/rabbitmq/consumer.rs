use lapin::{
    options::*, types::FieldTable, Channel,   Consumer,  Queue 
};
use futures_lite::stream::StreamExt;

use super::{RabbitMQCommon, RabbitMQConfig, RabbitMQError};

pub struct RabbitMQConsumer {
    common: RabbitMQCommon,
    queue: Queue,
    consumer: Consumer,
}

impl RabbitMQConsumer {
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let common = RabbitMQCommon::new(config).await?;
        
        // Passively declare the exchange (it should already exist)
        common.declare_exchange(config, true).await?;
        
        // Declare queue and bind it to the channel
        let queue = Self::declare_queue(&common.channel, config).await?;
        Self::bind_queue(&common.channel, &config.exchange, &queue, config).await?;
        
        // Initialize the consumer
        let consumer = Self::initialize_consumer(&common.channel, &config).await?;

        Ok(Self { common, queue, consumer})
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

    pub async fn consume(&self) -> Result<String, RabbitMQError> {
        let delivery = self.consumer.clone().next().await
            .ok_or_else(|| RabbitMQError::ConsumeError("No message received".to_string()))?
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        let message = String::from_utf8_lossy(&delivery.data).to_string();

        self.common.channel
            .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
            .await
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        Ok(message)
    }
}

