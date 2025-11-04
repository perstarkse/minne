mod pipeline;

pub use pipeline::run_evaluation;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{system_settings::SystemSettings, user::User},
    },
};
use composite_retrieval::pipeline as retrieval_pipeline;
use composite_retrieval::pipeline::PipelineStageTimings;
use composite_retrieval::pipeline::RetrievalTuning;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::{
    args::{self, Config},
    datasets::{self, ConvertedDataset},
    db_helpers::change_embedding_length_in_hnsw_indexes,
    ingest,
    slice::{self},
    snapshot::{self, DbSnapshotState},
};

#[derive(Debug, Serialize)]
pub struct EvaluationSummary {
    pub generated_at: DateTime<Utc>,
    pub k: usize,
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_label: Option<String>,
    pub total_cases: usize,
    pub correct: usize,
    pub precision: f64,
    pub correct_at_1: usize,
    pub correct_at_2: usize,
    pub correct_at_3: usize,
    pub precision_at_1: f64,
    pub precision_at_2: f64,
    pub precision_at_3: f64,
    pub duration_ms: u128,
    pub dataset_id: String,
    pub dataset_label: String,
    pub dataset_includes_unanswerable: bool,
    pub dataset_source: String,
    pub slice_id: String,
    pub slice_seed: u64,
    pub slice_total_cases: usize,
    pub slice_window_offset: usize,
    pub slice_window_length: usize,
    pub slice_cases: usize,
    pub slice_positive_paragraphs: usize,
    pub slice_negative_paragraphs: usize,
    pub slice_total_paragraphs: usize,
    pub slice_negative_multiplier: f32,
    pub namespace_reused: bool,
    pub corpus_paragraphs: usize,
    pub ingestion_cache_path: String,
    pub ingestion_reused: bool,
    pub ingestion_embeddings_reused: bool,
    pub ingestion_fingerprint: String,
    pub positive_paragraphs_reused: usize,
    pub negative_paragraphs_reused: usize,
    pub latency_ms: LatencyStats,
    pub perf: PerformanceTimings,
    pub embedding_backend: String,
    pub embedding_model: Option<String>,
    pub embedding_dimension: usize,
    pub rerank_enabled: bool,
    pub rerank_pool_size: Option<usize>,
    pub rerank_keep_top: usize,
    pub concurrency: usize,
    pub detailed_report: bool,
    pub chunk_vector_take: usize,
    pub chunk_fts_take: usize,
    pub chunk_token_budget: usize,
    pub chunk_avg_chars_per_token: usize,
    pub max_chunks_per_entity: usize,
    pub cases: Vec<CaseSummary>,
}

#[derive(Debug, Serialize)]
pub struct CaseSummary {
    pub question_id: String,
    pub question: String,
    pub paragraph_id: String,
    pub paragraph_title: String,
    pub expected_source: String,
    pub answers: Vec<String>,
    pub matched: bool,
    pub entity_match: bool,
    pub chunk_text_match: bool,
    pub chunk_id_match: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_rank: Option<usize>,
    pub latency_ms: u128,
    pub retrieved: Vec<RetrievedSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub avg: f64,
    pub p50: u128,
    pub p95: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct StageLatencyBreakdown {
    pub collect_candidates: LatencyStats,
    pub graph_expansion: LatencyStats,
    pub chunk_attach: LatencyStats,
    pub rerank: LatencyStats,
    pub assemble: LatencyStats,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct EvaluationStageTimings {
    pub prepare_slice_ms: u128,
    pub prepare_db_ms: u128,
    pub prepare_corpus_ms: u128,
    pub prepare_namespace_ms: u128,
    pub run_queries_ms: u128,
    pub summarize_ms: u128,
    pub finalize_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct PerformanceTimings {
    pub openai_base_url: String,
    pub ingestion_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_seed_ms: Option<u128>,
    pub evaluation_stage_ms: EvaluationStageTimings,
    pub stage_latency: StageLatencyBreakdown,
}

#[derive(Debug, Serialize)]
pub struct RetrievedSummary {
    pub rank: usize,
    pub entity_id: String,
    pub source_id: String,
    pub entity_name: String,
    pub score: f32,
    pub matched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_text_match: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id_match: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CaseDiagnostics {
    question_id: String,
    question: String,
    paragraph_id: String,
    paragraph_title: String,
    expected_source: String,
    expected_chunk_ids: Vec<String>,
    answers: Vec<String>,
    entity_match: bool,
    chunk_text_match: bool,
    chunk_id_match: bool,
    failure_reasons: Vec<String>,
    missing_expected_chunk_ids: Vec<String>,
    attached_chunk_ids: Vec<String>,
    retrieved: Vec<EntityDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pipeline: Option<retrieval_pipeline::PipelineDiagnostics>,
}

#[derive(Debug, Serialize)]
struct EntityDiagnostics {
    rank: usize,
    entity_id: String,
    source_id: String,
    name: String,
    score: f32,
    entity_match: bool,
    chunk_text_match: bool,
    chunk_id_match: bool,
    chunks: Vec<ChunkDiagnosticsEntry>,
}

#[derive(Debug, Serialize)]
struct ChunkDiagnosticsEntry {
    chunk_id: String,
    score: f32,
    contains_answer: bool,
    expected_chunk: bool,
    snippet: String,
}

pub(crate) struct SeededCase {
    question_id: String,
    question: String,
    expected_source: String,
    answers: Vec<String>,
    paragraph_id: String,
    paragraph_title: String,
    expected_chunk_ids: Vec<String>,
}

pub(crate) fn cases_from_manifest(manifest: &ingest::CorpusManifest) -> Vec<SeededCase> {
    let mut title_map = HashMap::new();
    for paragraph in &manifest.paragraphs {
        title_map.insert(paragraph.paragraph_id.as_str(), paragraph.title.clone());
    }

    manifest
        .questions
        .iter()
        .filter(|question| !question.is_impossible)
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
            }
        })
        .collect()
}

pub(crate) fn text_contains_answer(text: &str, answers: &[String]) -> bool {
    if answers.is_empty() {
        return true;
    }
    let haystack = text.to_ascii_lowercase();
    answers.iter().any(|needle| haystack.contains(needle))
}

pub(crate) fn compute_latency_stats(latencies: &[u128]) -> LatencyStats {
    if latencies.is_empty() {
        return LatencyStats {
            avg: 0.0,
            p50: 0,
            p95: 0,
        };
    }
    let mut sorted = latencies.to_vec();
    sorted.sort_unstable();
    let sum: u128 = sorted.iter().copied().sum();
    let avg = sum as f64 / (sorted.len() as f64);
    let p50 = percentile(&sorted, 0.50);
    let p95 = percentile(&sorted, 0.95);
    LatencyStats { avg, p50, p95 }
}

pub(crate) fn build_stage_latency_breakdown(
    samples: &[PipelineStageTimings],
) -> StageLatencyBreakdown {
    fn collect_stage<F>(samples: &[PipelineStageTimings], selector: F) -> Vec<u128>
    where
        F: Fn(&PipelineStageTimings) -> u128,
    {
        samples.iter().map(selector).collect()
    }

    StageLatencyBreakdown {
        collect_candidates: compute_latency_stats(&collect_stage(samples, |entry| {
            entry.collect_candidates_ms
        })),
        graph_expansion: compute_latency_stats(&collect_stage(samples, |entry| {
            entry.graph_expansion_ms
        })),
        chunk_attach: compute_latency_stats(&collect_stage(samples, |entry| entry.chunk_attach_ms)),
        rerank: compute_latency_stats(&collect_stage(samples, |entry| entry.rerank_ms)),
        assemble: compute_latency_stats(&collect_stage(samples, |entry| entry.assemble_ms)),
    }
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let clamped = fraction.clamp(0.0, 1.0);
    let idx = (clamped * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
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

pub(crate) fn apply_dataset_tuning_overrides(
    dataset: &ConvertedDataset,
    config: &Config,
    tuning: &mut RetrievalTuning,
) {
    let is_long_form = dataset
        .metadata
        .id
        .to_ascii_lowercase()
        .contains("natural-questions");
    if !is_long_form {
        return;
    }

    if config.chunk_vector_take.is_none() {
        tuning.chunk_vector_take = tuning.chunk_vector_take.max(80);
    }
    if config.chunk_fts_take.is_none() {
        tuning.chunk_fts_take = tuning.chunk_fts_take.max(80);
    }
    if config.chunk_token_budget.is_none() {
        tuning.token_budget_estimate = tuning.token_budget_estimate.max(20_000);
    }
    if config.max_chunks_per_entity.is_none() {
        tuning.max_chunks_per_entity = tuning.max_chunks_per_entity.max(12);
    }
    if tuning.lexical_match_weight < 0.25 {
        tuning.lexical_match_weight = 0.3;
    }
}

pub(crate) fn build_case_diagnostics(
    summary: &CaseSummary,
    expected_chunk_ids: &[String],
    answers_lower: &[String],
    entities: &[composite_retrieval::RetrievedEntity],
    pipeline_stats: Option<retrieval_pipeline::PipelineDiagnostics>,
) -> CaseDiagnostics {
    let expected_set: HashSet<&str> = expected_chunk_ids.iter().map(|id| id.as_str()).collect();
    let mut seen_chunks: HashSet<String> = HashSet::new();
    let mut attached_chunk_ids = Vec::new();
    let mut entity_diagnostics = Vec::new();

    for (idx, entity) in entities.iter().enumerate() {
        let mut chunk_entries = Vec::new();
        for chunk in &entity.chunks {
            let contains_answer = text_contains_answer(&chunk.chunk.chunk, answers_lower);
            let expected_chunk = expected_set.contains(chunk.chunk.id.as_str());
            seen_chunks.insert(chunk.chunk.id.clone());
            attached_chunk_ids.push(chunk.chunk.id.clone());
            chunk_entries.push(ChunkDiagnosticsEntry {
                chunk_id: chunk.chunk.id.clone(),
                score: chunk.score,
                contains_answer,
                expected_chunk,
                snippet: chunk_preview(&chunk.chunk.chunk),
            });
        }
        entity_diagnostics.push(EntityDiagnostics {
            rank: idx + 1,
            entity_id: entity.entity.id.clone(),
            source_id: entity.entity.source_id.clone(),
            name: entity.entity.name.clone(),
            score: entity.score,
            entity_match: entity.entity.source_id == summary.expected_source,
            chunk_text_match: chunk_entries.iter().any(|entry| entry.contains_answer),
            chunk_id_match: chunk_entries.iter().any(|entry| entry.expected_chunk),
            chunks: chunk_entries,
        });
    }

    let missing_expected_chunk_ids = expected_chunk_ids
        .iter()
        .filter(|id| !seen_chunks.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    let mut failure_reasons = Vec::new();
    if !summary.entity_match {
        failure_reasons.push("entity_miss".to_string());
    }
    if !summary.chunk_text_match {
        failure_reasons.push("chunk_text_missing".to_string());
    }
    if !summary.chunk_id_match {
        failure_reasons.push("chunk_id_missing".to_string());
    }
    if !missing_expected_chunk_ids.is_empty() {
        failure_reasons.push("expected_chunk_absent".to_string());
    }

    CaseDiagnostics {
        question_id: summary.question_id.clone(),
        question: summary.question.clone(),
        paragraph_id: summary.paragraph_id.clone(),
        paragraph_title: summary.paragraph_title.clone(),
        expected_source: summary.expected_source.clone(),
        expected_chunk_ids: expected_chunk_ids.to_vec(),
        answers: summary.answers.clone(),
        entity_match: summary.entity_match,
        chunk_text_match: summary.chunk_text_match,
        chunk_id_match: summary.chunk_id_match,
        failure_reasons,
        missing_expected_chunk_ids,
        attached_chunk_ids,
        retrieved: entity_diagnostics,
        pipeline: pipeline_stats,
    }
}

fn chunk_preview(text: &str) -> String {
    text.chars()
        .take(200)
        .collect::<String>()
        .replace('\n', " ")
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

    // Warm up chunk index
    let _ = db
        .client
        .query("SELECT * FROM text_chunk WHERE embedding <|1,1|> $embedding LIMIT 5")
        .bind(("embedding", dummy_embedding.clone()))
        .await
        .context("warming text chunk HNSW cache")?;

    // Warm up entity index
    let _ = db
        .client
        .query("SELECT * FROM knowledge_entity WHERE embedding <|1,1|> $embedding LIMIT 5")
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

    if let Some(existing) = db.get_item::<User>(&user.id).await? {
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

    if state.slice_case_count < slice_case_count {
        info!(
            requested_cases = slice_case_count,
            stored_cases = state.slice_case_count,
            "Skipping live namespace reuse; ledger grew beyond cached state"
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
    let mut embedding_dimension_changed = false;

    if provider_dimension != settings.embedding_dimensions as usize {
        updated_settings.embedding_dimensions = provider_dimension as u32;
        needs_settings_update = true;
        embedding_dimension_changed = true;
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
    if embedding_dimension_changed {
        change_embedding_length_in_hnsw_indexes(db, provider_dimension)
            .await
            .context("redefining HNSW indexes for new embedding dimension")?;
    }
    Ok(settings)
}

pub(crate) async fn load_or_init_system_settings(
    db: &SurrealDbClient,
) -> Result<(SystemSettings, bool)> {
    match SystemSettings::get_current(db).await {
        Ok(settings) => Ok((settings, false)),
        Err(AppError::NotFound(_)) => {
            info!("System settings missing; applying database migrations for namespace");
            db.apply_migrations()
                .await
                .context("applying database migrations after missing system settings")?;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let settings = SystemSettings::get_current(db)
                .await
                .context("loading system settings after migrations")?;
            Ok((settings, true))
        }
        Err(err) => Err(err).context("loading system settings"),
    }
}
