use lapin::{
    options::*, types::FieldTable, BasicProperties, Channel, Connection, ConnectionProperties, Result
};

pub struct RabbitMQClient {
    pub connection: Connection,
    pub channel: Channel,
}

impl RabbitMQClient {
    pub async fn new(addr: &str) -> Result<Self> {
        let connection = Connection::connect(addr, ConnectionProperties::default()).await?;
        let channel = connection.create_channel().await?;

        Ok(Self { connection, channel })
    }

    pub async fn publish(&self, queue: &str, payload: &[u8]) -> Result<()> {
        self.channel
            .basic_publish(
                "",
                queue,
                Default::default(),
                payload,
                BasicProperties::default(),
            )
            .await?;

        Ok(())
    }

    pub async fn consume(&self, queue: &str) -> Result<lapin::Consumer> {
        let consumer = self.channel
            .basic_consume(
                queue,
                "consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;

        Ok(consumer)
    }
}
