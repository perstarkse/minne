pub mod consumer;
pub mod publisher;

use axum::async_trait;
use lapin::{
    options::ExchangeDeclareOptions, types::FieldTable, Channel, Connection, ConnectionProperties,
    ExchangeKind,
};
use thiserror::Error;
use tracing::debug;

/// Possible errors related to RabbitMQ operations.
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

/// Struct containing the information required to set up a client and connection.
#[derive(Clone)]
pub struct RabbitMQConfig {
    pub amqp_addr: String,
    pub exchange: String,
    pub queue: String,
    pub routing_key: String,
}

/// Struct containing the connection and channel of a client
pub struct RabbitMQCommon {
    pub connection: Connection,
    pub channel: Channel,
}

/// Defines the behavior for RabbitMQCommon client operations.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RabbitMQCommonTrait: Send + Sync {
    async fn create_connection(config: &RabbitMQConfig) -> Result<Connection, RabbitMQError>;
    async fn declare_exchange(
        &self,
        config: &RabbitMQConfig,
        passive: bool,
    ) -> Result<(), RabbitMQError>;
}

impl RabbitMQCommon {
    /// Sets up a new RabbitMQ client or error
    ///
    /// # Arguments
    /// * `RabbitMQConfig` - Configuration object with required information
    ///
    /// # Returns
    /// * `self` - A initialized instance of the client
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let connection = Self::create_connection(config).await?;
        let channel = connection.create_channel().await?;
        Ok(Self {
            connection,
            channel,
        })
    }
}

#[async_trait]
impl RabbitMQCommonTrait for RabbitMQCommon {
    /// Function to set up the connection
    async fn create_connection(config: &RabbitMQConfig) -> Result<Connection, RabbitMQError> {
        debug!("Creating connection");
        Connection::connect(&config.amqp_addr, ConnectionProperties::default())
            .await
            .map_err(RabbitMQError::ConnectionError)
    }

    /// Function to declare the exchange required
    async fn declare_exchange(
        &self,
        config: &RabbitMQConfig,
        passive: bool,
    ) -> Result<(), RabbitMQError> {
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
