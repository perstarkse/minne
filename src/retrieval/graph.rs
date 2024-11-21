use surrealdb::{engine::remote::ws::Client, Surreal};

use crate::error::ProcessingError;

/// Retrieves database entries that match a specific source identifier.
///
/// This function queries the database for all records in a specified table that have
/// a matching `source_id` field. It's commonly used to find related entities or
/// track the origin of database entries.
///
/// # Arguments
///
/// * `source_id` - The identifier to search for in the database
/// * `table_name` - The name of the table to search in
/// * `db_client` - The SurrealDB client instance for database operations
///
/// # Type Parameters
///
/// * `T` - The type to deserialize the query results into. Must implement `serde::Deserialize`
///
/// # Returns
///
/// Returns a `Result` containing either:
/// * `Ok(Vec<T>)` - A vector of matching records deserialized into type `T`
/// * `Err(ProcessingError)` - An error if the database query fails
///
/// # Errors
///
/// This function will return a `ProcessingError` if:
/// * The database query fails to execute
/// * The results cannot be deserialized into type `T`
///
/// # Example
///
/// ```rust
/// #[derive(serde::Deserialize)]
/// struct KnowledgeEntity {
///     id: String,
///     source_id: String,
///     // ... other fields
/// }
///
/// let results = find_entities_by_source_id::<KnowledgeEntity>(
///     "source123".to_string(),
///     "knowledge_entity".to_string(),
///     &db_client
/// ).await?;
/// ```
pub async fn find_entities_by_source_id<T>(
    source_id: String,
    table_name: String,
    db_client: &Surreal<Client>,
) -> Result<Vec<T>, ProcessingError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let query = format!(
        "SELECT * FROM {} WHERE source_id = '{}'",
        table_name, source_id
    );

    let matching_entities: Vec<T> = db_client.query(query).await?.take(0)?;

    Ok(matching_entities)
}
