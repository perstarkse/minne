use crate::{
    error::ApiError,
    retrieval::{
        graph::{
            find_entities_by_relationship_by_id, find_entities_by_relationship_by_source_ids,
            find_entities_by_source_ids,
        },
        vector::find_items_by_vector_similarity,
    },
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk},
    },
};
use axum::{response::IntoResponse, Extension, Json};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
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

    let test = find_entities_by_relationship_by_id(&db_client, &query.query).await?;
    info!("{:?}", test);

    let items_from_knowledge_entity_similarity: Vec<KnowledgeEntity> =
        find_items_by_vector_similarity(
            10,
            query.query.to_string(),
            &db_client,
            "knowledge_entity".to_string(),
            &openai_client,
        )
        .await?;

    let closest_chunks: Vec<TextChunk> = find_items_by_vector_similarity(
        5,
        query.query,
        &db_client,
        "text_chunk".to_string(),
        &openai_client,
    )
    .await?;

    let source_ids = closest_chunks
        .iter()
        .map(|chunk| chunk.source_id.clone())
        .collect::<Vec<String>>();

    let items_from_text_chunk_similarity: Vec<KnowledgeEntity> = find_entities_by_source_ids(
        source_ids.clone(),
        "knowledge_entity".to_string(),
        &db_client,
    )
    .await?;

    let entities: Vec<KnowledgeEntity> = items_from_knowledge_entity_similarity
        .into_iter()
        .chain(items_from_text_chunk_similarity.into_iter())
        .fold(HashMap::new(), |mut map, entity| {
            map.insert(entity.id.clone(), entity);
            map
        })
        .into_values()
        .collect();

    let entities_json = json!(entities
        .iter()
        .map(|entity| {
            json!({
                "KnowledgeEntity": {
                    "id": entity.id,
                    "name": entity.name,
                    "description": entity.description
                }
            })
        })
        .collect::<Vec<_>>());

    let graph_retrieval =
        find_entities_by_relationship_by_source_ids(&db_client, &source_ids).await?;

    info!("{:?}", graph_retrieval);

    // info!("{} Entities\n{:#?}", entities.len(), entities_json);

    Ok("we got some stuff".to_string())
}
