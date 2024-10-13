use serde::{Deserialize, Serialize};
use surrealdb::{engine::remote::ws::Client, RecordId, Surreal};
use tracing::info;

use crate::models::file_info::FileInfo;

#[derive(Debug, Deserialize)]
struct Record {
    id: RecordId,
}

use super::SurrealError;

pub async fn set_file_info(client: Surreal<Client>, sha256: &str, file_info: FileInfo) -> Result<(), SurrealError> {
    info!("Creating in surrealdb");
    info!("{:?}, {:?}", sha256, file_info);

    // Use create instead of upsert if you're sure the record doesn't exist
    let created: Option<Record> = client
        .create(("file", sha256))
        .content(file_info)
        .await?;

    // If you want to update or create, use upsert instead
    // let created: Option<Record> = client
    //     .upsert(("file", sha256))
    //     .content(file_info)
    //     .await?;

    info!("{:?}", created);

    Ok(())
}
