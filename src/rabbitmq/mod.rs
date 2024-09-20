pub mod producer;
pub mod consumer;

use lapin::{
    Connection, ConnectionProperties, Channel, ExchangeKind,
    options::ExchangeDeclareOptions,
    types::FieldTable,
     
};
use thiserror::Error;
use tracing::debug;

#[derive(Error, Debug)]
pub enum RabbitMQError {
    #[error("Failed to connect to RabbitMQ: {0}")]
    ConnectionError(#[from] lapin::Error),
    #[error("Channel error: {0}")]
    ChannelError(String),
    #[error("Consume error: {0}")]
    ConsumeError(String),
    #[error("Exchange error: {0}")]
    ExchangeError(String),
    #[error("Publish error: {0}")]
    PublishError(String),
    #[error("Error initializing consumer: {0}")]
    InitializeConsumerError(String),
    #[error("Queue error: {0}")]
    QueueError(String),
}

pub struct RabbitMQConfig {
    pub amqp_addr: String,
    pub exchange: String,
    pub queue: String,
    pub routing_key: String,
}

pub struct RabbitMQCommon {
    pub connection: Connection,
    pub channel: Channel,
}

impl RabbitMQCommon {
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let connection = Self::create_connection(config).await?;
        let channel = connection.create_channel().await?;
        Ok(Self { connection, channel })
    }

    async fn create_connection(config: &RabbitMQConfig) -> Result<Connection, RabbitMQError> {
        debug!("Creating connection");
        Connection::connect(&config.amqp_addr, ConnectionProperties::default())
            .await
            .map_err(RabbitMQError::ConnectionError)
    }

    pub async fn declare_exchange(&self, config: &RabbitMQConfig, passive: bool) -> Result<(), RabbitMQError> {
        debug!("Declaring exchange");
        self.channel
            .exchange_declare(
                &config.exchange,
                ExchangeKind::Topic,
                ExchangeDeclareOptions {
                    passive,
                    durable: true,
                    ..ExchangeDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| RabbitMQError::ExchangeError(e.to_string()))
    }
}

