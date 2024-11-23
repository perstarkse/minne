use crate::{
    error::ApiError,
    ingress::types::ingress_input::{create_ingress_objects, IngressInput},
    rabbitmq::publisher::RabbitMQProducer,
    storage::db::SurrealDbClient,
};
use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use std::sync::Arc;
use tracing::info;

pub async fn ingress_handler(
    Extension(producer): Extension<Arc<RabbitMQProducer>>,
    Extension(db_client): Extension<Arc<SurrealDbClient>>,
    Json(input): Json<IngressInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", input);

    let ingress_objects = create_ingress_objects(input, &db_client).await?;

    for object in ingress_objects {
        producer.publish(&object).await?;
    }

    Ok(StatusCode::OK)
}
