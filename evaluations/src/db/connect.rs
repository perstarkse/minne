use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use common::{
    storage::{
        db::SurrealDbClient,
        types::user::{Theme, User},
        types::StoredObject,
    },
    utils::embedding::EmbeddingProvider,
};
use serde::Deserialize;
use tracing::{info, warn};

use crate::{
    args::Config,
    corpus::{self, CorpusHandle, CorpusManifest, NamespaceSeedRecord},
    datasets,
};

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
    Ok(rows.first().map_or(0, |row| row.count) > 0)
}

fn manifest_matches_runtime(
    manifest: &CorpusManifest,
    embedding_provider: &EmbeddingProvider,
    ingestion_fingerprint: &str,
) -> bool {
    let metadata = &manifest.metadata;
    metadata.ingestion_fingerprint == ingestion_fingerprint
        && metadata.embedding_backend == embedding_provider.backend_label()
        && metadata.embedding_model == embedding_provider.model_code()
        && metadata.embedding_dimension == embedding_provider.dimension()
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn can_reuse_namespace(
    db: &SurrealDbClient,
    manifest: &CorpusManifest,
    embedding_provider: &EmbeddingProvider,
    namespace: &str,
    database: &str,
    ingestion_fingerprint: &str,
    slice_case_count: usize,
) -> Result<bool> {
    if !manifest_matches_runtime(manifest, embedding_provider, ingestion_fingerprint) {
        info!("Corpus manifest metadata mismatch; rebuilding namespace from cached shards");
        return Ok(false);
    }

    let Some(seed) = manifest.metadata.namespace_seed.as_ref() else {
        info!("No namespace seed recorded in corpus manifest; reseeding");
        return Ok(false);
    };

    if seed.slice_case_count != slice_case_count {
        info!(
            requested_cases = slice_case_count,
            stored_cases = seed.slice_case_count,
            "Skipping namespace reuse; case window mismatch"
        );
        return Ok(false);
    }

    if seed.namespace != namespace || seed.database != database {
        info!(
            namespace,
            database, "Corpus manifest namespace metadata mismatch; reseeding"
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

pub(crate) async fn record_namespace_seed(
    handle: &mut CorpusHandle,
    namespace: &str,
    database: &str,
    slice_case_count: usize,
) {
    handle.manifest.metadata.namespace_seed = Some(NamespaceSeedRecord {
        namespace: namespace.to_string(),
        database: database.to_string(),
        slice_case_count,
        seeded_at: Utc::now(),
    });
    if let Err(err) = corpus::persist_corpus_manifest(handle) {
        warn!(error = %err, "Failed to record namespace seed in corpus manifest");
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

pub(crate) fn default_namespace(
    dataset_id: &str,
    limit: Option<usize>,
    slice_id: Option<&str>,
) -> String {
    if let Some(slice_id) = slice_id {
        let sanitized = sanitize_identifier(slice_id);
        if !sanitized.is_empty() {
            return format!("eval_{sanitized}");
        }
    }
    let dataset_component = sanitize_identifier(dataset_id);
    let limit_component = match limit {
        Some(value) if value > 0 => format!("limit{value}"),
        _ => "all".to_string(),
    };
    format!("eval_{dataset_component}_{limit_component}")
}

pub(crate) fn default_database() -> String {
    "retrieval_eval".to_string()
}

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
        theme: Theme::System,
    };

    if let Some(existing) = db.get_item::<User>(user.id()).await? {
        return Ok(existing);
    }

    db.store_item(user.clone())
        .await
        .context("storing evaluation user")?;
    Ok(user)
}

pub(crate) fn sanitize_model_code(code: &str) -> String {
    sanitize_identifier(code)
}
