use amqprs::{
    callbacks::{DefaultChannelCallback, DefaultConnectionCallback},
    channel::{
        BasicConsumeArguments, BasicPublishArguments, QueueBindArguments, QueueDeclareArguments,
    },
    connection::{Connection, OpenConnectionArguments},
    consumer::DefaultConsumer,
    BasicProperties,
};

pub struct RabbitMQ {
    pub connection: Connection,
    pub channel: amqprs::channel::Channel,
}

impl RabbitMQ {
    pub async fn new() -> Self {
        let connection = Connection::open(&OpenConnectionArguments::new(
            "localhost",
            5672,
            "user",
            "bitnami",
        ))
        .await
        .unwrap();

        connection
            .register_callback(DefaultConnectionCallback)
            .await
            .unwrap();

        let channel = connection.open_channel(None).await.unwrap();
        channel
            .register_callback(DefaultChannelCallback)
            .await
            .unwrap();

        RabbitMQ { connection, channel }
    }

    pub async fn declare_queue(&self, queue_name: &str) -> (String, u32, u32) {
        self.channel
            .queue_declare(QueueDeclareArguments::durable_client_named(queue_name))
            .await
            .unwrap()
            .unwrap()
    }

    pub async fn bind_queue(&self, queue_name: &str, exchange_name: &str, routing_key: &str) {
        self.channel
            .queue_bind(QueueBindArguments::new(queue_name, exchange_name, routing_key))
            .await
            .unwrap();
    }
}

