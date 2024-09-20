// use lapin::{
//     options::*, types::FieldTable, BasicProperties, Channel, Connection, ConnectionProperties, Result
// };

// pub struct RabbitMQClient {
//     pub connection: Connection,
//     pub channel: Channel,
// }

// impl RabbitMQClient {
//     pub async fn new(addr: &str) -> Result<Self> {
//         let connection = Connection::connect(addr, ConnectionProperties::default()).await?;
//         let channel = connection.create_channel().await?;

//         Ok(Self { connection, channel })
//     }

//     pub async fn publish(&self, queue: &str, payload: &[u8]) -> Result<()> {
//         self.channel
//             .basic_publish(
//                 "",
//                 queue,
//                 Default::default(),
//                 payload,
//                 BasicProperties::default(),
//             )
//             .await?;

//         Ok(())
//     }

//     pub async fn consume(&self, queue: &str) -> Result<lapin::Consumer> {
//         let consumer = self.channel
//             .basic_consume(
//                 queue,
//                 "consumer",
//                 BasicConsumeOptions::default(),
//                 FieldTable::default(),
//             )
//             .await?;

//         Ok(consumer)
//     }
// }
use lapin::{
    options::*, types::FieldTable, BasicProperties, Channel, Connection, ConnectionProperties, Consumer, ExchangeKind 
};
use futures_lite::stream::StreamExt;

#[derive(Debug, thiserror::Error)]
pub enum RabbitMQError {
    #[error("Connection error: {0}")]
    ConnectionError(#[from] lapin::Error),
    #[error("Channel error: {0}")]
    ChannelError(String),
    #[error("Consume error: {0}")]
    ConsumeError(String),
    #[error("Publish error: {0}")]
    PublishError(String),
}

struct RabbitMQConnection {
    pub connection: Connection,
    pub channel: Channel,
}

impl RabbitMQConnection {
    async fn new(url: &str) -> Result<Self, RabbitMQError> {
        let connection = Connection::connect(
            url,
            ConnectionProperties::default(),
        ).await?;
        
        let channel = connection.create_channel().await?;
        
        Ok(Self { connection, channel })
    }
}

// pub struct RabbitMQConsumer {
//     connection: RabbitMQConnection,
//     queue_name: String,
// }

// impl RabbitMQConsumer {
//     pub async fn new(url: &str, queue_name: &str) -> Result<Self, RabbitMQError> {
//         let connection = RabbitMQConnection::new(url).await?;
//         Ok(Self {
//             connection,
//             queue_name: queue_name.to_string(),
//         })
//     }

//     pub async fn consume(&self) -> Result<String, RabbitMQError> {
//         let consumer = self.connection.channel
//             .basic_consume(
//                 &self.queue_name,
//                 "consumer",
//                 BasicConsumeOptions::default(),
//                 FieldTable::default(),
//             )
//             .await
//             .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

//         let delivery = consumer.clone().next().await
//             .ok_or_else(|| RabbitMQError::ConsumeError("No message received".to_string()))?
//             .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

//         let message = String::from_utf8_lossy(&delivery.data).to_string();

//         self.connection.channel
//             .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
//             .await
//             .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

//         Ok(message)
//     }
// }
pub struct RabbitMQConsumer {
    connection: Connection,
    channel: Channel,
    consumer: Consumer,
}

impl RabbitMQConsumer {
    pub async fn new(url: &str, exchange: &str, queue: &str, routing_key: &str) -> Result<Self, RabbitMQError> {
        let connection = Connection::connect(
            url,
            ConnectionProperties::default(),
        ).await?;
        
        let channel = connection.create_channel().await?;

        // Declare the exchange
        channel.exchange_declare(
            exchange,
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                auto_delete: false,
                ..ExchangeDeclareOptions::default()
            },
            FieldTable::default(),
        ).await?;

        // Declare the queue
        channel.queue_declare(
            queue,
            QueueDeclareOptions {
                durable: true,
                auto_delete: false,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        ).await?;

        // Bind the queue to the exchange
        channel.queue_bind(
            queue,
            exchange,
            routing_key,
            QueueBindOptions::default(),
            FieldTable::default(),
        ).await?;

        // Create the consumer
        let consumer = channel
            .basic_consume(
                queue,
                "consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;

        Ok(Self {
            connection,
            channel,
            consumer,
        })
    }

    pub async fn consume(&self) -> Result<String, RabbitMQError> {
        let delivery = self.consumer.clone().next().await
            .ok_or_else(|| RabbitMQError::ConsumeError("No message received".to_string()))?
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        let message = String::from_utf8_lossy(&delivery.data).to_string();

        self.channel
            .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
            .await
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        Ok(message)
    }
}


pub struct RabbitMQProducer {
    connection: Connection,
    channel: Channel,
    exchange_name: String,
    routing_key: String,
}

impl RabbitMQProducer {
    pub async fn new(url: &str, exchange_name: &str, routing_key: &str) -> Result<Self, RabbitMQError> {
        let connection = Connection::connect(
            url,
            ConnectionProperties::default(),
        ).await?;
        
        let channel = connection.create_channel().await?;

        // Declare the exchange
        channel.exchange_declare(
            exchange_name,
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                auto_delete: false,
                ..ExchangeDeclareOptions::default()
            },
            FieldTable::default(),
        ).await?;

        Ok(Self {
            connection,
            channel,
            exchange_name: exchange_name.to_string(),
            routing_key: routing_key.to_string(),
        })
    }

    pub async fn publish(&self, message: &str) -> Result<(), RabbitMQError> {
        self.channel
            .basic_publish(
                &self.exchange_name,
                &self.routing_key,
                BasicPublishOptions::default(),
                message.as_bytes(),
                BasicProperties::default(),
            )
            .await?
            .await?;

        Ok(())
    }
}
