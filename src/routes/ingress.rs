use crate::{
    ingress::types::ingress_input::{create_ingress_objects, IngressInput},
    rabbitmq::publisher::RabbitMQProducer,
    storage::db::SurrealDbClient,
};
use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use std::sync::Arc;
use tracing::{error, info};

pub async fn ingress_handler(
    Extension(producer): Extension<Arc<RabbitMQProducer>>,
    Extension(db_client): Extension<Arc<SurrealDbClient>>,
    Json(input): Json<IngressInput>,
) -> impl IntoResponse {
    info!("Received input: {:?}", input);

    match create_ingress_objects(input, &db_client).await {
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
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process input").into_response()
        }
    }
}
