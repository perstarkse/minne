use std::sync::Arc;

use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use tracing::{error, info};

use crate::{models::ingress::{IngressContent, IngressInput}, rabbitmq::publisher::RabbitMQProducer};

pub async fn ingress_handler(
    Extension(producer): Extension<Arc<RabbitMQProducer>>,
    Json(input): Json<IngressInput>,
) -> impl IntoResponse {
    info!("Recieved input: {:?}", input);

    if let Ok(content) = IngressContent::new(input).await {

    // Publish content to RabbitMQ (or other system)
    match producer.publish(&content).await {
        Ok(_) => {
            info!("Message published successfully");
            "Successfully processed".to_string().into_response()
        }
        Err(e) => {
            error!("Failed to publish message: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish message").into_response()
        }
    }
    }
    else {
        error!("Failed to create IngressContent object" );
        (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create object").into_response()
        
    }
}
