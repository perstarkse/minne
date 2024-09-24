use axum::{
        extract::Multipart, http::StatusCode, response::{IntoResponse, Response}, routing::{get, post}, Extension, Json, Router
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;
use zettle_db::{models::ingress::{FileInfo, IngressContent, IngressMultipart  }, rabbitmq::{consumer::RabbitMQConsumer, RabbitMQConfig}};
use zettle_db::rabbitmq::publisher::RabbitMQProducer;
use std::sync::Arc;
use axum_typed_multipart::TypedMultipart;
use axum::debug_handler;

use tracing::{info, error};

pub async fn ingress_handler(
    Extension(producer): Extension<Arc<RabbitMQProducer>>,
    TypedMultipart(multipart_data): TypedMultipart<IngressMultipart>, // Parse form data
) -> impl IntoResponse {
    info!("Received multipart data: {:?}", &multipart_data);

    let file_info = if let Some(file) = multipart_data.file {
        // File name or default to "data.bin" if none is provided
        let file_name = file.metadata.file_name.unwrap_or(String::from("data.bin"));
        let mime_type = mime_guess::from_path(&file_name)
            .first_or_octet_stream()
            .to_string();
        let uuid = Uuid::new_v4();
        let path = std::path::Path::new("/tmp").join(uuid.to_string()).join(&file_name);

        // Persist the file
        match file.contents.persist(&path) {
            Ok(_) => {
                info!("File saved at: {:?}", path);
                // Generate FileInfo
                let file_info = FileInfo {
                    uuid,
                    sha256: "sha-12412".to_string(),
                    path: path.to_string_lossy().to_string(),
                    mime_type,
                };
                Some(file_info)
            }
            Err(e) => {
                error!("Failed to save file: {:?}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to store file").into_response();
            }
        }
    } else {
        None // No file was uploaded
    };

    // Convert `IngressMultipart` to `IngressContent`
    let content = match IngressContent::new(multipart_data.content, multipart_data.instructions,multipart_data.category, file_info).await {
        Ok(content) => content,
        Err(e) => {
            error!("Error creating IngressContent: {:?}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create content").into_response();
        }
    };

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


async fn queue_length_handler() -> Response {
    info!("Getting queue length");
    
    // Set up RabbitMQ config
    let config = RabbitMQConfig {
        amqp_addr: "amqp://localhost".to_string(),
        exchange: "my_exchange".to_string(),
        queue: "my_queue".to_string(),
        routing_key: "my_key".to_string(),
    };

    // Create a new consumer
    match RabbitMQConsumer::new(&config).await {
        Ok(consumer) => {
            info!("Consumer connected to RabbitMQ");

            // Get the queue length
            let queue_length = consumer.queue.message_count();

            info!("Queue length: {}", queue_length);

            // Return the queue length with a 200 OK status
            (StatusCode::OK, queue_length.to_string()).into_response()
        },
        Err(e) => {
            error!("Failed to create consumer: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to connect to RabbitMQ".to_string()).into_response()
        }
    }
}


#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
       .with(fmt::layer())
       .with(EnvFilter::from_default_env())
       .try_init()
       .ok();

    // Set up RabbitMQ
    let config = RabbitMQConfig {
        amqp_addr: "amqp://localhost".to_string(),
        exchange: "my_exchange".to_string(),
        queue: "my_queue".to_string(),
        routing_key: "my_key".to_string(),
    };
    
    let producer = Arc::new(RabbitMQProducer::new(&config).await?);
    
    // Create Axum router
    let app = Router::new()
        .route("/ingress", post(ingress_handler))
        .route("/message_count", get(queue_length_handler))
        .layer(Extension(producer));

    tracing::info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}


