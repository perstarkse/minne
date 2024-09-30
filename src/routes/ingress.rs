use std::sync::Arc;
use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use tracing::{error, info};
use crate::{models::ingress_content::{create_ingress_objects, IngressInput}, rabbitmq::publisher::RabbitMQProducer, redis::client::RedisClient};

pub async fn ingress_handler(
    Extension(producer): Extension<Arc<RabbitMQProducer>>,
    Json(input): Json<IngressInput>,
) -> impl IntoResponse {
    info!("Received input: {:?}", input);

    let redis_client = RedisClient::new("redis://127.0.0.1/");

    match create_ingress_objects(input, &redis_client).await {
        Ok(objects) => {
            for object in objects {
                match producer.publish(&object).await {
                    Ok(_) => {
                        info!("Message published successfully");
                    }
                    Err(e) => {
                        error!("Failed to publish message: {:?}", e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to publish message",
                        )
                        .into_response();
                    }
                }
            }
            StatusCode::OK.into_response()
        }
        Err(e) => {
            error!("Failed to process input: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to process input",
            )
            .into_response()
        }
    }
}
