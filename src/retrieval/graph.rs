use surrealdb::{engine::remote::ws::Client, Surreal};
use tracing::debug;

use crate::{
    error::ProcessingError,
    storage::types::{knowledge_entity::KnowledgeEntity, StoredObject},
};

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

// pub async fn find_entities_by_relationship_by_source_ids(
//     db_client: &Surreal<Client>,
//     source_ids: &[String],
// ) -> Result<Vec<KnowledgeEntity>, ProcessingError> {
//     let ids = source_ids
//         .iter()
//         .map(|id| format!("knowledge_entity:`{}`", id))
//         .collect::<Vec<_>>()
//         .join(", ");

//     debug!("{:?}", ids);

//     let query = format!(
//         "SELECT *, <-> relates_to <-> knowledge_entity AS related FROM [{}]",
//         ids
//     );

//     debug!("{}", query);

//     let result: Vec<KnowledgeEntity> = db_client.query(query).await?.take(0)?;

//     Ok(result)
// }

/// Find entities by their relationship to the id
pub async fn find_entities_by_relationship_by_id(
    db_client: &Surreal<Client>,
    entity_id: String,
) -> Result<Vec<KnowledgeEntity>, ProcessingError> {
    let query = format!(
        "SELECT *, <-> relates_to <-> knowledge_entity AS related FROM knowledge_entity:`{}`",
        entity_id
    );

    debug!("{}", query);

    let result: Vec<KnowledgeEntity> = db_client.query(query).await?.take(0)?;

    Ok(result)
}

/// Get a specific KnowledgeEntity by its id
pub async fn get_entity_by_id(
    db_client: &Surreal<Client>,
    entity_id: &str,
) -> Result<Option<KnowledgeEntity>, ProcessingError> {
    let response: Option<KnowledgeEntity> = db_client
        .select((KnowledgeEntity::table_name(), entity_id))
        .await?;

    Ok(response)
}

pub async fn get_all_stored_items<T>(db_client: &Surreal<Client>) -> Result<Vec<T>, ProcessingError>
where
    T: for<'de> StoredObject,
{
    let response: Vec<T> = db_client.select(T::table_name()).await?;
    Ok(response)
}
