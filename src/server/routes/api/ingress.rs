use crate::{
    error::{ApiError, AppError},
    ingress::types::ingress_input::{create_ingress_objects, IngressInput},
    server::AppState,
    storage::types::user::User,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use futures::future::try_join_all;
use tracing::info;

pub async fn ingress_handler(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Json(input): Json<IngressInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", input);

    let ingress_objects = create_ingress_objects(input, &state.surreal_db_client, &user.id).await?;

    let futures: Vec<_> = ingress_objects
        .into_iter()
        .map(|object| state.rabbitmq_producer.publish(object))
        .collect();

    try_join_all(futures).await.map_err(AppError::from)?;

    Ok(StatusCode::OK)
}
