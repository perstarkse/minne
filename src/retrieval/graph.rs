use surrealdb::{engine::remote::ws::Client, Surreal};
use tracing::info;

use crate::{error::ProcessingError, storage::types::knowledge_entity::KnowledgeEntity};

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
pub async fn find_entities_by_source_ids<T>(
    source_id: Vec<String>,
    table_name: String,
    db_client: &Surreal<Client>,
) -> Result<Vec<T>, ProcessingError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let query = "SELECT * FROM type::table($table) WHERE source_id IN $source_ids";

    let matching_entities: Vec<T> = db_client
        .query(query)
        .bind(("table", table_name))
        .bind(("source_ids", source_id))
        .await?
        .take(0)?;

    Ok(matching_entities)
}

pub async fn find_entities_by_relationship_by_source_ids(
    db_client: &Surreal<Client>,
    source_ids: &[String],
) -> Result<Vec<KnowledgeEntity>, ProcessingError> {
    // Create a comma-separated list of IDs wrapped in backticks
    let ids = source_ids
        .iter()
        .map(|id| format!("knowledge_entity:`{}`", id))
        .collect::<Vec<_>>()
        .join(", ");

    info!("{:?}", ids);

    // let first = format!("knowledge_entity:`{}`", source_ids.first().unwrap());

    let query = format!(
        "SELECT *, array::complement(<->relates_to<->knowledge_entity, [id]) AS related FROM [{}] FETCH related",
        ids
    );

    info!("{}", query);

    let result: Vec<KnowledgeEntity> = db_client.query(query).await?.take(0)?;

    Ok(result)
}
pub async fn find_entities_by_relationship_by_id(
    db_client: &Surreal<Client>,
    source_id: &str,
) -> Result<Vec<KnowledgeEntity>, ProcessingError> {
    let query = format!(
        "SELECT *, <-> relates_to <-> knowledge_entity AS related FROM knowledge_entity:`{}`",
        source_id
    );

    info!("{}", query);

    let result: Vec<KnowledgeEntity> = db_client.query(query).await?.take(0)?;

    Ok(result)
}
