use std::collections::HashMap;

use common::storage::types::file_info::deserialize_flexible_id;
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::StoredObject},
    utils::embedding::generate_embedding,
};
use serde::Deserialize;
use surrealdb::sql::Thing;

use crate::scoring::{clamp_unit, distance_to_similarity, Scored};

/// Compares vectors and retrieves a number of items from the specified table.
///
/// This function generates embeddings for the input text, constructs a query to find the closest matches in the database,
/// and then deserializes the results into the specified type `T`.
///
/// # Arguments
///
/// * `take` - The number of items to retrieve from the database.
/// * `input_text` - The text to generate embeddings for.
/// * `db_client` - The SurrealDB client to use for querying the database.
/// * `table` - The table to query in the database.
/// * `openai_client` - The OpenAI client to use for generating embeddings.
/// * 'user_id`-  The user id of the current user.
///
/// # Returns
///
/// A vector of type `T` containing the closest matches to the input text. Returns a `ProcessingError` if an error occurs.
///
/// # Type Parameters
///
/// * `T` - The type to deserialize the query results into. Must implement `serde::Deserialize`.
pub async fn find_items_by_vector_similarity<T>(
    take: usize,
    input_text: &str,
    db_client: &SurrealDbClient,
    table: &str,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    user_id: &str,
) -> Result<Vec<Scored<T>>, AppError>
where
    T: for<'de> serde::Deserialize<'de> + StoredObject,
{
    // Generate embeddings
    let input_embedding = generate_embedding(openai_client, input_text, db_client).await?;
    find_items_by_vector_similarity_with_embedding(take, input_embedding, db_client, table, user_id)
        .await
}

#[derive(Debug, Deserialize)]
struct DistanceRow {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    id: String,
    distance: Option<f32>,
}

pub async fn find_items_by_vector_similarity_with_embedding<T>(
    take: usize,
    query_embedding: Vec<f32>,
    db_client: &SurrealDbClient,
    table: &str,
    user_id: &str,
) -> Result<Vec<Scored<T>>, AppError>
where
    T: for<'de> serde::Deserialize<'de> + StoredObject,
{
    let embedding_literal = serde_json::to_string(&query_embedding)
        .map_err(|err| AppError::InternalError(format!("Failed to serialize embedding: {err}")))?;
    let closest_query = format!(
        "SELECT id, vector::distance::knn() AS distance \
         FROM {table} \
         WHERE user_id = $user_id AND embedding <|{take},40|> {embedding} \
         LIMIT $limit",
        table = table,
        take = take,
        embedding = embedding_literal
    );

    let mut response = db_client
        .query(closest_query)
        .bind(("user_id", user_id.to_owned()))
        .bind(("limit", take as i64))
        .await?;

    let distance_rows: Vec<DistanceRow> = response.take(0)?;

    if distance_rows.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = distance_rows.iter().map(|row| row.id.clone()).collect();
    let thing_ids: Vec<Thing> = ids
        .iter()
        .map(|id| Thing::from((table, id.as_str())))
        .collect();

    let mut items_response = db_client
        .query("SELECT * FROM type::table($table) WHERE id IN $things AND user_id = $user_id")
        .bind(("table", table.to_owned()))
        .bind(("things", thing_ids.clone()))
        .bind(("user_id", user_id.to_owned()))
        .await?;

    let items: Vec<T> = items_response.take(0)?;

    let mut item_map: HashMap<String, T> = items
        .into_iter()
        .map(|item| (item.get_id().to_owned(), item))
        .collect();

    let mut min_distance = f32::MAX;
    let mut max_distance = f32::MIN;

    for row in &distance_rows {
        if let Some(distance) = row.distance {
            if distance.is_finite() {
                if distance < min_distance {
                    min_distance = distance;
                }
                if distance > max_distance {
                    max_distance = distance;
                }
            }
        }
    }

    let normalize = min_distance.is_finite()
        && max_distance.is_finite()
        && (max_distance - min_distance).abs() > f32::EPSILON;

    let mut scored = Vec::with_capacity(distance_rows.len());
    for row in distance_rows {
        if let Some(item) = item_map.remove(&row.id) {
            let similarity = row
                .distance
                .map(|distance| {
                    if normalize {
                        let span = max_distance - min_distance;
                        if span.abs() < f32::EPSILON {
                            1.0
                        } else {
                            let normalized = 1.0 - ((distance - min_distance) / span);
                            clamp_unit(normalized)
                        }
                    } else {
                        distance_to_similarity(distance)
                    }
                })
                .unwrap_or_default();
            scored.push(Scored::new(item).with_vector_score(similarity));
        }
    }

    Ok(scored)
}
