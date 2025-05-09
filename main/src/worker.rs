use std::sync::Arc;

use common::{storage::db::SurrealDbClient, utils::config::get_config};
use ingestion_pipeline::{pipeline::IngestionPipeline, run_worker_loop};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = get_config()?;

    let db = Arc::new(
        SurrealDbClient::new(
            &config.surrealdb_address,
            &config.surrealdb_username,
            &config.surrealdb_password,
            &config.surrealdb_namespace,
            &config.surrealdb_database,
        )
        .await?,
    );

    let openai_client = Arc::new(async_openai::Client::new());

    let ingestion_pipeline =
        Arc::new(IngestionPipeline::new(db.clone(), openai_client.clone(), config).await?);

    run_worker_loop(db, ingestion_pipeline).await
}
