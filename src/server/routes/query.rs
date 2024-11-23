use crate::{
    error::ApiError,
    retrieval::vector::find_items_by_vector_similarity,
    storage::{db::SurrealDbClient, types::knowledge_entity::KnowledgeEntity},
};
use axum::{response::IntoResponse, Extension, Json};
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Deserialize)]
pub struct QueryInput {
    query: String,
}

pub async fn query_handler(
    Extension(db_client): Extension<Arc<SurrealDbClient>>,
    Json(query): Json<QueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", query);
    let openai_client = async_openai::Client::new();

    let closest_items: Vec<KnowledgeEntity> = find_items_by_vector_similarity(
        10,
        query.query,
        &db_client,
        "knowledge_entity".to_string(),
        &openai_client,
    )
    .await?;

    Ok(format!("{:?}", closest_items))
}
