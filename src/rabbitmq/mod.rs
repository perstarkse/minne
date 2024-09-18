use amqprs::{
    callbacks::{DefaultChannelCallback, DefaultConnectionCallback},
    channel::{
        BasicConsumeArguments, BasicPublishArguments, QueueBindArguments, QueueDeclareArguments,
    },
    connection::{Connection, OpenConnectionArguments},
    consumer::DefaultConsumer,
    BasicProperties,
};
use tokio::sync::mpsc;

pub struct RabbitMQ {
    pub connection: Connection,
    pub channel: amqprs::channel::Channel,
}

impl RabbitMQ {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let connection = Connection::open(&OpenConnectionArguments::new(
            "localhost",
            5672,
            "user",
            "bitnami",
        ))
        .await?;

        connection
            .register_callback(DefaultConnectionCallback)
            .await?;

        let channel = connection.open_channel(None).await?;
        channel
            .register_callback(DefaultChannelCallback)
            .await?;

        Ok(RabbitMQ { connection, channel })
    }

    pub async fn declare_queue(&self, queue_name: &str) -> Result<(String, u32, u32), Box<dyn std::error::Error>> {
        Ok(self.channel
            .queue_declare(QueueDeclareArguments::durable_client_named(queue_name))
            .await??
        )
    }

    pub async fn bind_queue(&self, queue_name: &str, exchange_name: &str, routing_key: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.channel
            .queue_bind(QueueBindArguments::new(queue_name, exchange_name, routing_key))
            .await?;
        Ok(())
    }

    pub async fn publish_message(&self, exchange_name: &str, routing_key: &str, content: String) -> Result<(), Box<dyn std::error::Error>> {
        let args = BasicPublishArguments::new(exchange_name, routing_key);
        self.channel
            .basic_publish(BasicProperties::default(), content.into_bytes(), args)
            .await?;
        Ok(())
    }

    pub async fn consume_messages(&self, queue_name: &str, consumer_tag: &str) -> Result<mpsc::Receiver<String>, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel(100);
        let args = BasicConsumeArguments::new(queue_name, consumer_tag);
        
        let consumer = DefaultConsumer::new(args.no_ack).with_callback(move |_deliver, _basic_properties, content| {
            let content_str = String::from_utf8_lossy(&content).to_string();
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = tx.send(content_str).await {
                    eprintln!("Failed to send message: {}", e);
                }
            });
        });

        self.channel.basic_consume(consumer, args).await?;
        Ok(rx)
    }
}

