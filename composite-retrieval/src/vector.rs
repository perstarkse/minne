use common::{error::AppError, storage::db::SurrealDbClient, utils::embedding::generate_embedding};

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
    take: u8,
    input_text: &str,
    db_client: &SurrealDbClient,
    table: &str,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    user_id: &str,
) -> Result<Vec<T>, AppError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    // Generate embeddings
    let input_embedding = generate_embedding(openai_client, input_text, db_client).await?;

    // Construct the query
    let closest_query = format!("SELECT *, vector::distance::knn() AS distance FROM {} WHERE user_id = '{}' AND embedding <|{},40|> {:?} ORDER BY distance", table, user_id, take, input_embedding);

    // Perform query and deserialize to struct
    let closest_entities: Vec<T> = db_client.query(closest_query).await?.take(0)?;

    Ok(closest_entities)
}
