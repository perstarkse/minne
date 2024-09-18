use zettle_db::rabbitmq::RabbitMQ;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let rabbitmq = RabbitMQ::new().await;
    let queue_name = rabbitmq.declare_queue("amqprs.examples.basic").await;
    rabbitmq.bind_queue(&queue_name.0, "amq.topic", "amqprs.example").await;
    //...
}
