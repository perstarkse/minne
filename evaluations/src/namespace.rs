//! Database namespace management utilities.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use common::storage::{db::SurrealDbClient, types::user::User, types::StoredObject};
use serde::Deserialize;
use tracing::{info, warn};

use crate::{
    args::Config,
    datasets,
    snapshot::{self, DbSnapshotState},
};

/// Connect to the evaluation database with fallback auth strategies.
pub(crate) async fn connect_eval_db(
    config: &Config,
    namespace: &str,
    database: &str,
) -> Result<SurrealDbClient> {
    match SurrealDbClient::new(
        &config.database.db_endpoint,
        &config.database.db_username,
        &config.database.db_password,
        namespace,
        database,
    )
    .await
    {
        Ok(client) => {
            info!(
                endpoint = %config.database.db_endpoint,
                namespace,
                database,
                auth = "root",
                "Connected to SurrealDB"
            );
            Ok(client)
        }
        Err(root_err) => {
            info!(
                endpoint = %config.database.db_endpoint,
                namespace,
                database,
                "Root authentication failed; trying namespace-level auth"
            );
            let namespace_client = SurrealDbClient::new_with_namespace_user(
                &config.database.db_endpoint,
                namespace,
                &config.database.db_username,
                &config.database.db_password,
                database,
            )
            .await
            .map_err(|ns_err| {
                anyhow!(
                    "failed to connect to SurrealDB via root ({root_err}) or namespace ({ns_err}) credentials"
                )
            })?;
            info!(
                endpoint = %config.database.db_endpoint,
                namespace,
                database,
                auth = "namespace",
                "Connected to SurrealDB"
            );
            Ok(namespace_client)
        }
    }
}

/// Check if the namespace contains any corpus data.
pub(crate) async fn namespace_has_corpus(db: &SurrealDbClient) -> Result<bool> {
    #[derive(Deserialize)]
    struct CountRow {
        count: i64,
    }

    let mut response = db
        .client
        .query("SELECT count() AS count FROM text_chunk")
        .await
        .context("checking namespace corpus state")?;
    let rows: Vec<CountRow> = response.take(0).unwrap_or_default();
    Ok(rows.first().map(|row| row.count).unwrap_or(0) > 0)
}

/// Determine if we can reuse an existing namespace based on cached state.
pub(crate) async fn can_reuse_namespace(
    db: &SurrealDbClient,
    descriptor: &snapshot::Descriptor,
    namespace: &str,
    database: &str,
    dataset_id: &str,
    slice_id: &str,
    ingestion_fingerprint: &str,
    slice_case_count: usize,
) -> Result<bool> {
    let state = match descriptor.load_db_state().await? {
        Some(state) => state,
        None => {
            info!("No namespace state recorded; reseeding corpus from cached shards");
            return Ok(false);
        }
    };

    if state.slice_case_count != slice_case_count {
        info!(
            requested_cases = slice_case_count,
            stored_cases = state.slice_case_count,
            "Skipping live namespace reuse; cached state does not match requested window"
        );
        return Ok(false);
    }

    if state.dataset_id != dataset_id
        || state.slice_id != slice_id
        || state.ingestion_fingerprint != ingestion_fingerprint
        || state.namespace.as_deref() != Some(namespace)
        || state.database.as_deref() != Some(database)
    {
        info!(
            namespace,
            database, "Cached namespace metadata mismatch; rebuilding corpus from ingestion cache"
        );
        return Ok(false);
    }

    if namespace_has_corpus(db).await? {
        Ok(true)
    } else {
        info!(
            namespace,
            database,
            "Namespace metadata matches but tables are empty; reseeding from ingestion cache"
        );
        Ok(false)
    }
}

/// Record the current namespace state to allow future reuse checks.
pub(crate) async fn record_namespace_state(
    descriptor: &snapshot::Descriptor,
    dataset_id: &str,
    slice_id: &str,
    ingestion_fingerprint: &str,
    namespace: &str,
    database: &str,
    slice_case_count: usize,
) {
    let state = DbSnapshotState {
        dataset_id: dataset_id.to_string(),
        slice_id: slice_id.to_string(),
        ingestion_fingerprint: ingestion_fingerprint.to_string(),
        snapshot_hash: descriptor.metadata_hash().to_string(),
        updated_at: Utc::now(),
        namespace: Some(namespace.to_string()),
        database: Some(database.to_string()),
        slice_case_count,
    };
    if let Err(err) = descriptor.store_db_state(&state).await {
        warn!(error = %err, "Failed to record namespace state");
    }
}

fn sanitize_identifier(input: &str) -> String {
    let mut cleaned: String = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        cleaned.push('x');
    }
    if cleaned.len() > 64 {
        cleaned.truncate(64);
    }
    cleaned
}

/// Generate a default namespace name based on dataset and limit.
pub(crate) fn default_namespace(dataset_id: &str, limit: Option<usize>) -> String {
    let dataset_component = sanitize_identifier(dataset_id);
    let limit_component = match limit {
        Some(value) if value > 0 => format!("limit{}", value),
        _ => "all".to_string(),
    };
    format!("eval_{}_{}", dataset_component, limit_component)
}

/// Generate the default database name for evaluations.
pub(crate) fn default_database() -> String {
    "retrieval_eval".to_string()
}

/// Ensure the evaluation user exists in the database.
pub(crate) async fn ensure_eval_user(db: &SurrealDbClient) -> Result<User> {
    let timestamp = datasets::base_timestamp();
    let user = User {
        id: "eval-user".to_string(),
        created_at: timestamp,
        updated_at: timestamp,
        email: "eval-retrieval@minne.dev".to_string(),
        password: "not-used".to_string(),
        anonymous: false,
        api_key: None,
        admin: false,
        timezone: "UTC".to_string(),
    };

    if let Some(existing) = db.get_item::<User>(&user.get_id()).await? {
        return Ok(existing);
    }

    db.store_item(user.clone())
        .await
        .context("storing evaluation user")?;
    Ok(user)
}
