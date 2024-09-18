use zettle_db::rabbitmq::RabbitMQ;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rabbitmq = RabbitMQ::new().await?;
    let queue_name = rabbitmq.declare_queue("amqprs.examples.basic").await?.0;
    rabbitmq.bind_queue(&queue_name, "amq.topic", "amqprs.example").await?;

    let mut rx = rabbitmq.consume_messages(&queue_name, "example_consumer").await?;

    println!("Consumer waiting for messages. To exit press CTRL+C");

    while let Some(message) = rx.recv().await {
        println!("Received message: {}", message);
    }

    Ok(())
}
