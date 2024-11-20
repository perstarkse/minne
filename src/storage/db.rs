use surrealdb::{engine::remote::ws::Client, Surreal};

use crate::error::ProcessingError;

use super::types::StoredObject;

/// Operation to store a object in SurrealDB, requires the struct to implement StoredObject
///
/// # Arguments
/// * `db_client` - A initialized database client
/// * `item` - The item to be stored
///
/// # Returns
/// * `Result` - Item or Error
pub async fn store_item<T>(
    db_client: &Surreal<Client>,
    item: T,
) -> Result<Option<T>, ProcessingError>
where
    T: StoredObject + Send + Sync + 'static,
{
    db_client
        .create((T::table_name(), item.get_id()))
        .content(item)
        .await
        .map_err(ProcessingError::from)
}
