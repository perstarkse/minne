use crate::storage::db::SurrealDbClient;
use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Debug, Deserialize)]
pub struct QueryInput {
    query: String,
}

pub async fn query_handler(
    Extension(db_client): Extension<Arc<SurrealDbClient>>,
    Json(query): Json<QueryInput>,
) -> impl IntoResponse {
    info!("Received input: {:?}", query);
}
