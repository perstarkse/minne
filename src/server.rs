// use axum::{
//     routing::post,
//     Router,
//     response::{IntoResponse, Response},
//     Error, Extension, Json,
// };
// use serde::Deserialize;
// use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// #[derive(Deserialize)]
// struct IngressPayload {
//     payload: String,
// }

// use tracing::{info, error};
// use zettle_db::rabbitmq::RabbitMQClient;

// async fn ingress_handler(
//     // Extension(rabbitmq): Extension<RabbitMQ>,
//     Json(payload): Json<IngressPayload>
// ) -> Response {
//     info!("Received payload: {:?}", payload.payload);
//     let rabbitmqclient = RabbitMQClient::new("127.0.0.1").await;
//     match rabbitmq.publish(&payload.payload).await {
//         Ok(_) => {
//             info!("Message published successfully");
//             "thank you".to_string().into_response()
//         },
//         Err(e) => {
//             error!("Failed to publish message: {:?}", e);
//             (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish message").into_response()
//         }
//     }
// }

// #[tokio::main(flavor = "multi_thread", worker_threads = 2)]
// async fn main() -> Result<(), Error> {
//     // Set up tracing
//     tracing_subscriber::registry()
//        .with(fmt::layer())
//        .with(EnvFilter::from_default_env())
//        .try_init()
//        .ok();

//     // Set up RabbitMQ
//     // let rabbitmq = RabbitMQ::new("amqprs.examples.basic2", "amq.topic", "amqprs.example2").await;

//     // Create Axum router
//     let app = Router::new()
//         .route("/ingress", post(ingress_handler));        // .layer(Extension(rabbitmq));

//     tracing::info!("Listening on 0.0.0.0:3000");
//     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//     axum::serve(listener, app).await.unwrap();

//     Ok(())
// }
use axum::{
    routing::post,
    Router,
    response::{IntoResponse, Response},
    Error, Extension, Json,
};
use serde::Deserialize;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zettle_db::rabbitmq::RabbitMQClient;
use std::sync::Arc;

#[derive(Deserialize)]
struct IngressPayload {
    payload: String,
}

use tracing::{info, error};

async fn ingress_handler(
    Extension(rabbitmq): Extension<Arc<RabbitMQClient>>,
    Json(payload): Json<IngressPayload>
) -> Response {
    info!("Received payload: {:?}", payload.payload);
    match rabbitmq.publish("hello", payload.payload.as_bytes()).await {
        Ok(_) => {
            info!("Message published successfully");
            "thank you".to_string().into_response()
        },
        Err(e) => {
            error!("Failed to publish message: {:?}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish message").into_response()
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Error> {
    // Set up tracing
    tracing_subscriber::registry()
       .with(fmt::layer())
       .with(EnvFilter::from_default_env())
       .try_init()
       .ok();

    // Set up RabbitMQ
    let rabbitmq = Arc::new(RabbitMQClient::new("amqp://guest:guest@localhost:5672").await.expect("Failed to connect to RabbitMQ"));
    
    // Create Axum router
    let app = Router::new()
        .route("/ingress", post(ingress_handler))
        .layer(Extension(rabbitmq));

    tracing::info!("Listening on 0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

