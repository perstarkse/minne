pub mod answer_retrieval;
pub mod answer_retrieval_helper;
pub mod graph;
pub mod vector;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk},
    },
};
use futures::future::{try_join, try_join_all};
use graph::{find_entities_by_relationship_by_id, find_entities_by_source_ids};
use std::collections::HashMap;
use tracing::info;
use vector::find_items_by_vector_similarity;

/// Performs a comprehensive knowledge entity retrieval using multiple search strategies
/// to find the most relevant entities for a given query.
///
/// # Strategy
/// The function employs a three-pronged approach to knowledge retrieval:
/// 1. Direct vector similarity search on knowledge entities
/// 2. Text chunk similarity search with source entity lookup
/// 3. Graph relationship traversal from related entities
///
/// This combined approach ensures both semantic similarity matches and structurally
/// related content are included in the results.
///
/// # Arguments
/// * `db_client` - SurrealDB client for database operations
/// * `openai_client` - OpenAI client for vector embeddings generation
/// * `query` - The search query string to find relevant knowledge entities
/// * 'user_id' - The user id of the current user
///
/// # Returns
/// * `Result<Vec<KnowledgeEntity>, AppError>` - A deduplicated vector of relevant
///   knowledge entities, or an error if the retrieval process fails
pub async fn retrieve_entities(
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    query: &str,
    user_id: &str,
) -> Result<Vec<KnowledgeEntity>, AppError> {
    let (items_from_knowledge_entity_similarity, closest_chunks) = try_join(
        find_items_by_vector_similarity(
            10,
            query,
            db_client,
            "knowledge_entity",
            openai_client,
            user_id,
        ),
        find_items_by_vector_similarity(5, query, db_client, "text_chunk", openai_client, user_id),
    )
    .await?;

    let source_ids = closest_chunks
        .iter()
        .map(|chunk: &TextChunk| chunk.source_id.clone())
        .collect::<Vec<String>>();

    let items_from_text_chunk_similarity: Vec<KnowledgeEntity> =
        find_entities_by_source_ids(source_ids, "knowledge_entity".to_string(), db_client).await?;

    let items_from_relationships_futures: Vec<_> = items_from_text_chunk_similarity
        .clone()
        .into_iter()
        .map(|entity| find_entities_by_relationship_by_id(db_client, entity.id.clone()))
        .collect();

    let items_from_relationships = try_join_all(items_from_relationships_futures)
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<KnowledgeEntity>>();

    let entities: Vec<KnowledgeEntity> = items_from_knowledge_entity_similarity
        .into_iter()
        .chain(items_from_text_chunk_similarity.into_iter())
        .chain(items_from_relationships.into_iter())
        .fold(HashMap::new(), |mut map, entity| {
            map.insert(entity.id.clone(), entity);
            map
        })
        .into_values()
        .collect();

    Ok(entities)
}
