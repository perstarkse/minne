use surrealdb::Error;
use tracing::debug;

use crate::storage::{db::SurrealDbClient, types::knowledge_entity::KnowledgeEntity};

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
/// * `Err(Error)` - An error if the database query fails
///
/// # Errors
///
/// This function will return a `Error` if:
/// * The database query fails to execute
/// * The results cannot be deserialized into type `T`
pub async fn find_entities_by_source_ids<T>(
    source_id: Vec<String>,
    table_name: String,
    db: &SurrealDbClient,
) -> Result<Vec<T>, Error>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let query = "SELECT * FROM type::table($table) WHERE source_id IN $source_ids";

    db.query(query)
        .bind(("table", table_name))
        .bind(("source_ids", source_id))
        .await?
        .take(0)
}

/// Find entities by their relationship to the id
pub async fn find_entities_by_relationship_by_id(
    db: &SurrealDbClient,
    entity_id: String,
) -> Result<Vec<KnowledgeEntity>, Error> {
    let query = format!(
        "SELECT *, <-> relates_to <-> knowledge_entity AS related FROM knowledge_entity:`{}`",
        entity_id
    );

    debug!("{}", query);

    db.query(query).await?.take(0)
}
