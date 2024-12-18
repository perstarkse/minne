use crate::rabbitmq::consumer::RabbitMQConsumer;
use crate::rabbitmq::publisher::RabbitMQProducer;
use crate::storage::db::SurrealDbClient;
use minijinja_autoreload::AutoReloader;
use std::sync::Arc;

pub mod middleware_api_auth;
pub mod routes;

#[derive(Clone)]
pub struct AppState {
    pub rabbitmq_producer: Arc<RabbitMQProducer>,
    pub rabbitmq_consumer: Arc<RabbitMQConsumer>,
    pub surreal_db_client: Arc<SurrealDbClient>,
    pub openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    pub templates: Arc<AutoReloader>,
}
