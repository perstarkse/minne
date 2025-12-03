mod pipeline;
mod types;

pub use pipeline::run_evaluation;
pub use types::*;

use std::{collections::HashMap, path::Path};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{system_settings::SystemSettings, user::User, StoredObject},
    },
};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::{
    args::{self, Config},
    datasets::{self, ConvertedDataset},
    ingest,
    slice::{self},
    snapshot::{self, DbSnapshotState},
};

pub(crate) struct SeededCase {
    question_id: String,
    question: String,
    expected_source: String,
    answers: Vec<String>,
    paragraph_id: String,
    paragraph_title: String,
    expected_chunk_ids: Vec<String>,
    is_impossible: bool,
    has_verified_chunks: bool,
}

pub(crate) fn cases_from_manifest(manifest: &ingest::CorpusManifest) -> Vec<SeededCase> {
    let mut title_map = HashMap::new();
    for paragraph in &manifest.paragraphs {
        title_map.insert(paragraph.paragraph_id.as_str(), paragraph.title.clone());
    }

    let include_impossible = manifest.metadata.include_unanswerable;
    let require_verified_chunks = manifest.metadata.require_verified_chunks;

    manifest
        .questions
        .iter()
        .filter(|question| {
            should_include_question(question, include_impossible, require_verified_chunks)
        })
        .map(|question| {
            let title = title_map
                .get(question.paragraph_id.as_str())
                .cloned()
                .unwrap_or_else(|| "Untitled".to_string());
            SeededCase {
                question_id: question.question_id.clone(),
                question: question.question_text.clone(),
                expected_source: question.text_content_id.clone(),
                answers: question.answers.clone(),
                paragraph_id: question.paragraph_id.clone(),
                paragraph_title: title,
                expected_chunk_ids: question.matching_chunk_ids.clone(),
                is_impossible: question.is_impossible,
                has_verified_chunks: !question.matching_chunk_ids.is_empty(),
            }
        })
        .collect()
}

fn should_include_question(
    question: &ingest::CorpusQuestion,
    include_impossible: bool,
    require_verified_chunks: bool,
) -> bool {
    if !include_impossible && question.is_impossible {
        return false;
    }
    if require_verified_chunks && question.matching_chunk_ids.is_empty() {
        return false;
    }
    true
}

pub async fn grow_slice(dataset: &ConvertedDataset, config: &Config) -> Result<()> {
    let ledger_limit = ledger_target(config);
    let slice_settings = slice::slice_config_with_limit(config, ledger_limit);
    let slice =
        slice::resolve_slice(dataset, &slice_settings).context("resolving dataset slice")?;
    info!(
        slice = slice.manifest.slice_id.as_str(),
        cases = slice.manifest.case_count,
        positives = slice.manifest.positive_paragraphs,
        negatives = slice.manifest.negative_paragraphs,
        total_paragraphs = slice.manifest.total_paragraphs,
        "Slice ledger ready"
    );
    println!(
        "Slice `{}` now contains {} questions ({} positives, {} negatives)",
        slice.manifest.slice_id,
        slice.manifest.case_count,
        slice.manifest.positive_paragraphs,
        slice.manifest.negative_paragraphs
    );
    Ok(())
}

pub(crate) fn ledger_target(config: &Config) -> Option<usize> {
    match (config.slice_grow, config.limit) {
        (Some(grow), Some(limit)) => Some(limit.max(grow)),
        (Some(grow), None) => Some(grow),
        (None, limit) => limit,
    }
}

pub(crate) async fn write_chunk_diagnostics(path: &Path, cases: &[CaseDiagnostics]) -> Result<()> {
    args::ensure_parent(path)?;
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("creating diagnostics file {}", path.display()))?;
    for case in cases {
        let line = serde_json::to_vec(case).context("serialising chunk diagnostics entry")?;
        file.write_all(&line).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}

pub(crate) async fn warm_hnsw_cache(db: &SurrealDbClient, dimension: usize) -> Result<()> {
    // Create a dummy embedding for cache warming
    let dummy_embedding: Vec<f32> = (0..dimension).map(|i| (i as f32).sin()).collect();

    info!("Warming HNSW caches with sample queries");

    // Warm up chunk embedding index - just query the embedding table to load HNSW index
    let _ = db
        .client
        .query(
            r#"SELECT chunk_id
               FROM text_chunk_embedding
               WHERE embedding <|1,1|> $embedding
               LIMIT 5"#,
        )
        .bind(("embedding", dummy_embedding.clone()))
        .await
        .context("warming text chunk HNSW cache")?;

    // Warm up entity embedding index
    let _ = db
        .client
        .query(
            r#"SELECT entity_id
               FROM knowledge_entity_embedding
               WHERE embedding <|1,1|> $embedding
               LIMIT 5"#,
        )
        .bind(("embedding", dummy_embedding))
        .await
        .context("warming knowledge entity HNSW cache")?;

    info!("HNSW cache warming completed");
    Ok(())
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
    };

    if let Some(existing) = db.get_item::<User>(&user.get_id()).await? {
        return Ok(existing);
    }

    db.store_item(user.clone())
        .await
        .context("storing evaluation user")?;
    Ok(user)
}

pub fn format_timestamp(timestamp: &DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn sanitize_model_code(code: &str) -> String {
    code.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) async fn connect_eval_db(
    config: &Config,
    namespace: &str,
    database: &str,
) -> Result<SurrealDbClient> {
    match SurrealDbClient::new(
        &config.db_endpoint,
        &config.db_username,
        &config.db_password,
        namespace,
        database,
    )
    .await
    {
        Ok(client) => {
            info!(
                endpoint = %config.db_endpoint,
                namespace,
                database,
                auth = "root",
                "Connected to SurrealDB"
            );
            Ok(client)
        }
        Err(root_err) => {
            info!(
                endpoint = %config.db_endpoint,
                namespace,
                database,
                "Root authentication failed; trying namespace-level auth"
            );
            let namespace_client = SurrealDbClient::new_with_namespace_user(
                &config.db_endpoint,
                namespace,
                &config.db_username,
                &config.db_password,
                database,
            )
            .await
            .map_err(|ns_err| {
                anyhow!(
                    "failed to connect to SurrealDB via root ({root_err}) or namespace ({ns_err}) credentials"
                )
            })?;
            info!(
                endpoint = %config.db_endpoint,
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
    Ok(rows.first().map(|row| row.count).unwrap_or(0) > 0)
}

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

pub(crate) fn default_namespace(dataset_id: &str, limit: Option<usize>) -> String {
    let dataset_component = sanitize_identifier(dataset_id);
    let limit_component = match limit {
        Some(value) if value > 0 => format!("limit{}", value),
        _ => "all".to_string(),
    };
    format!("eval_{}_{}", dataset_component, limit_component)
}

pub(crate) fn default_database() -> String {
    "retrieval_eval".to_string()
}

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

pub(crate) async fn enforce_system_settings(
    db: &SurrealDbClient,
    mut settings: SystemSettings,
    provider_dimension: usize,
    config: &Config,
) -> Result<SystemSettings> {
    let mut updated_settings = settings.clone();
    let mut needs_settings_update = false;

    if provider_dimension != settings.embedding_dimensions as usize {
        updated_settings.embedding_dimensions = provider_dimension as u32;
        needs_settings_update = true;
    }
    if let Some(query_override) = config.query_model.as_deref() {
        if settings.query_model != query_override {
            info!(
                model = query_override,
                "Overriding system query model for this run"
            );
            updated_settings.query_model = query_override.to_string();
            needs_settings_update = true;
        }
    }
    if needs_settings_update {
        settings = SystemSettings::update(db, updated_settings)
            .await
            .context("updating system settings overrides")?;
    }
    Ok(settings)
}

pub(crate) async fn load_or_init_system_settings(
    db: &SurrealDbClient,
    _dimension: usize,
) -> Result<(SystemSettings, bool)> {
    match SystemSettings::get_current(db).await {
        Ok(settings) => Ok((settings, false)),
        Err(AppError::NotFound(_)) => {
            info!("System settings missing; applying database migrations for namespace");
            db.apply_migrations()
                .await
                .context("applying database migrations after missing system settings")?;
            let settings = SystemSettings::get_current(db)
                .await
                .context("loading system settings after migrations")?;
            Ok((settings, true))
        }
        Err(err) => Err(err).context("loading system settings"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::store::{CorpusParagraph, EmbeddedKnowledgeEntity, EmbeddedTextChunk};
    use crate::ingest::{CorpusManifest, CorpusMetadata, CorpusQuestion, MANIFEST_VERSION};
    use chrono::Utc;
    use common::storage::types::text_content::TextContent;

    fn sample_manifest() -> CorpusManifest {
        let paragraphs = vec![
            CorpusParagraph {
                paragraph_id: "p1".to_string(),
                title: "Alpha".to_string(),
                text_content: TextContent::new(
                    "alpha context".to_string(),
                    None,
                    "test".to_string(),
                    None,
                    None,
                    "user".to_string(),
                ),
                entities: Vec::<EmbeddedKnowledgeEntity>::new(),
                relationships: Vec::new(),
                chunks: Vec::<EmbeddedTextChunk>::new(),
            },
            CorpusParagraph {
                paragraph_id: "p2".to_string(),
                title: "Beta".to_string(),
                text_content: TextContent::new(
                    "beta context".to_string(),
                    None,
                    "test".to_string(),
                    None,
                    None,
                    "user".to_string(),
                ),
                entities: Vec::<EmbeddedKnowledgeEntity>::new(),
                relationships: Vec::new(),
                chunks: Vec::<EmbeddedTextChunk>::new(),
            },
        ];
        let questions = vec![
            CorpusQuestion {
                question_id: "q1".to_string(),
                paragraph_id: "p1".to_string(),
                text_content_id: "tc-alpha".to_string(),
                question_text: "What is Alpha?".to_string(),
                answers: vec!["Alpha".to_string()],
                is_impossible: false,
                matching_chunk_ids: vec!["chunk-alpha".to_string()],
            },
            CorpusQuestion {
                question_id: "q2".to_string(),
                paragraph_id: "p1".to_string(),
                text_content_id: "tc-alpha".to_string(),
                question_text: "Unanswerable?".to_string(),
                answers: Vec::new(),
                is_impossible: true,
                matching_chunk_ids: Vec::new(),
            },
            CorpusQuestion {
                question_id: "q3".to_string(),
                paragraph_id: "p2".to_string(),
                text_content_id: "tc-beta".to_string(),
                question_text: "Where is Beta?".to_string(),
                answers: vec!["Beta".to_string()],
                is_impossible: false,
                matching_chunk_ids: Vec::new(),
            },
        ];
        CorpusManifest {
            version: MANIFEST_VERSION,
            metadata: CorpusMetadata {
                dataset_id: "ds".to_string(),
                dataset_label: "Dataset".to_string(),
                slice_id: "slice".to_string(),
                include_unanswerable: true,
                require_verified_chunks: true,
                ingestion_fingerprint: "fp".to_string(),
                embedding_backend: "test".to_string(),
                embedding_model: None,
                embedding_dimension: 3,
                converted_checksum: "chk".to_string(),
                generated_at: Utc::now(),
                paragraph_count: paragraphs.len(),
                question_count: questions.len(),
                chunk_min_tokens: 1,
                chunk_max_tokens: 10,
                chunk_only: false,
            },
            paragraphs,
            questions,
        }
    }

    #[test]
    fn cases_respect_mode_filters() {
        let mut manifest = sample_manifest();
        manifest.metadata.include_unanswerable = false;
        manifest.metadata.require_verified_chunks = true;

        let strict_cases = cases_from_manifest(&manifest);
        assert_eq!(strict_cases.len(), 1);
        assert_eq!(strict_cases[0].question_id, "q1");
        assert_eq!(strict_cases[0].paragraph_title, "Alpha");

        let mut llm_manifest = manifest.clone();
        llm_manifest.metadata.include_unanswerable = true;
        llm_manifest.metadata.require_verified_chunks = false;

        let llm_cases = cases_from_manifest(&llm_manifest);
        let ids: Vec<_> = llm_cases
            .iter()
            .map(|case| case.question_id.as_str())
            .collect();
        assert_eq!(ids, vec!["q1", "q2", "q3"]);
    }
}
