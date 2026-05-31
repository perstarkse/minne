use std::time::Duration;

use anyhow::{Context, Result};
use futures::future::try_join_all;
use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::{debug, info, warn};

use crate::{error::AppError, storage::db::SurrealDbClient};

const INDEX_POLL_INTERVAL: Duration = Duration::from_millis(50);
const INDEX_BUILD_TIMEOUT: Duration = Duration::from_mins(30);
const FTS_ANALYZER_NAME: &str = "app_en_fts_analyzer";

/// HNSW index options used by runtime index creation (includes CONCURRENTLY).
pub const HNSW_INDEX_OPTIONS: &str = "DIST COSINE TYPE F32 EFC 100 M 8 CONCURRENTLY";
/// HNSW index options for use inside transactions (CONCURRENTLY not supported).
pub const HNSW_INDEX_OPTIONS_SYNC: &str = "DIST COSINE TYPE F32 EFC 100 M 8";

/// Builds a `DEFINE INDEX OVERWRITE ... HNSW` statement matching runtime index options.
#[must_use]
pub fn hnsw_index_overwrite_sql(index_name: &str, table: &str, dimension: usize) -> String {
    format!(
        "DEFINE INDEX OVERWRITE {index_name} ON TABLE {table} \
         FIELDS embedding HNSW DIMENSION {dimension} {HNSW_INDEX_OPTIONS};"
    )
}

/// Recreates an HNSW index inside a transaction (for tests and dimension migrations).
#[must_use]
pub fn hnsw_index_redefine_transaction_sql(
    index_name: &str,
    table: &str,
    dimension: usize,
) -> String {
    format!(
        "BEGIN TRANSACTION;
         REMOVE INDEX IF EXISTS {index_name} ON TABLE {table};
         DEFINE INDEX {index_name} ON TABLE {table} FIELDS embedding HNSW DIMENSION {dimension} {HNSW_INDEX_OPTIONS_SYNC};
         COMMIT TRANSACTION;"
    )
}

#[derive(Clone, Copy)]
struct HnswIndexSpec {
    index_name: &'static str,
    table: &'static str,
    options: &'static str,
}

const fn hnsw_index_specs() -> [HnswIndexSpec; 2] {
    [
        HnswIndexSpec {
            index_name: "idx_embedding_text_chunk_embedding",
            table: "text_chunk_embedding",
            options: HNSW_INDEX_OPTIONS,
        },
        HnswIndexSpec {
            index_name: "idx_embedding_knowledge_entity_embedding",
            table: "knowledge_entity_embedding",
            options: HNSW_INDEX_OPTIONS,
        },
    ]
}

const fn fts_index_specs() -> [FtsIndexSpec; 8] {
    [
        FtsIndexSpec {
            index_name: "text_content_fts_idx",
            table: "text_content",
            field: "text",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "text_content_context_fts_idx",
            table: "text_content",
            field: "context",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "text_content_file_name_fts_idx",
            table: "text_content",
            field: "file_info.file_name",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "text_content_url_fts_idx",
            table: "text_content",
            field: "url_info.url",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "text_content_url_title_fts_idx",
            table: "text_content",
            field: "url_info.title",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "knowledge_entity_fts_name_idx",
            table: "knowledge_entity",
            field: "name",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "knowledge_entity_fts_description_idx",
            table: "knowledge_entity",
            field: "description",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "text_chunk_fts_chunk_idx",
            table: "text_chunk",
            field: "chunk",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
    ]
}

impl HnswIndexSpec {
    fn definition_if_not_exists(&self, dimension: usize) -> String {
        format!(
            "DEFINE INDEX IF NOT EXISTS {index} ON TABLE {table} \
             FIELDS embedding HNSW DIMENSION {dimension} {options};",
            index = self.index_name,
            table = self.table,
            dimension = dimension,
            options = self.options,
        )
    }

    fn definition_overwrite(&self, dimension: usize) -> String {
        format!(
            "DEFINE INDEX OVERWRITE {index} ON TABLE {table} \
             FIELDS embedding HNSW DIMENSION {dimension} {options};",
            index = self.index_name,
            table = self.table,
            dimension = dimension,
            options = self.options,
        )
    }
}

#[derive(Clone, Copy)]
struct FtsIndexSpec {
    index_name: &'static str,
    table: &'static str,
    field: &'static str,
    analyzer: Option<&'static str>,
    method: &'static str,
}

impl FtsIndexSpec {
    fn definition(&self) -> String {
        let analyzer_clause = self
            .analyzer
            .map(|analyzer| format!(" SEARCH ANALYZER {analyzer} {}", self.method))
            .unwrap_or_default();

        format!(
            "DEFINE INDEX IF NOT EXISTS {index} ON TABLE {table} FIELDS {field}{analyzer_clause} CONCURRENTLY;",
            index = self.index_name,
            table = self.table,
            field = self.field,
        )
    }

    fn overwrite_definition(&self) -> String {
        let analyzer_clause = self
            .analyzer
            .map(|analyzer| format!(" SEARCH ANALYZER {analyzer} {}", self.method))
            .unwrap_or_default();

        format!(
            "DEFINE INDEX OVERWRITE {index} ON TABLE {table} FIELDS {field}{analyzer_clause} CONCURRENTLY;",
            index = self.index_name,
            table = self.table,
            field = self.field,
        )
    }
}

/// Build runtime Surreal indexes (FTS + HNSW) using concurrent creation with readiness polling.
/// Idempotent: safe to call multiple times and will overwrite HNSW definitions when the dimension changes.
///
/// # Errors
///
/// Returns `AppError::InternalError` if any index definition or polling step fails.
pub async fn ensure_runtime(
    db: &SurrealDbClient,
    embedding_dimension: usize,
) -> Result<(), AppError> {
    ensure_runtime_inner(db, embedding_dimension)
        .await
        .map_err(AppError::internal)
}

/// Rebuild known FTS and HNSW indexes, skipping any that are not yet defined.
///
/// # Errors
///
/// Returns `AppError::InternalError` if any index rebuild operation fails.
pub async fn rebuild(db: &SurrealDbClient) -> Result<(), AppError> {
    rebuild_inner(db).await.map_err(AppError::internal)
}

async fn ensure_runtime_inner(db: &SurrealDbClient, embedding_dimension: usize) -> Result<()> {
    create_fts_analyzer(db).await?;

    for spec in fts_index_specs() {
        if index_exists(db, spec.table, spec.index_name).await? {
            continue;
        }
        // We need to create these sequentially otherwise SurrealDB errors with read/write clash
        create_index_with_polling(
            db,
            spec.definition(),
            spec.index_name,
            spec.table,
            Some(spec.table),
        )
        .await?;
    }

    let hnsw_tasks = hnsw_index_specs().into_iter().map(|spec| async move {
        match hnsw_index_state(db, &spec, embedding_dimension).await? {
            HnswIndexState::Missing => {
                create_index_with_polling(
                    db,
                    spec.definition_if_not_exists(embedding_dimension),
                    spec.index_name,
                    spec.table,
                    Some(spec.table),
                )
                .await
            }
            HnswIndexState::Matches => {
                let status = get_index_status(db, spec.index_name, spec.table).await?;
                if status.eq_ignore_ascii_case("error") {
                    warn!(
                        index = spec.index_name,
                        table = spec.table,
                        "HNSW index found in error state; triggering rebuild"
                    );
                    create_index_with_polling(
                        db,
                        spec.definition_overwrite(embedding_dimension),
                        spec.index_name,
                        spec.table,
                        Some(spec.table),
                    )
                    .await
                } else {
                    Ok(())
                }
            }
            HnswIndexState::Different(existing) => {
                info!(
                    index = spec.index_name,
                    table = spec.table,
                    existing_dimension = existing,
                    target_dimension = embedding_dimension,
                    "Overwriting HNSW index to match new embedding dimension"
                );
                create_index_with_polling(
                    db,
                    spec.definition_overwrite(embedding_dimension),
                    spec.index_name,
                    spec.table,
                    Some(spec.table),
                )
                .await
            }
        }
    });

    try_join_all(hnsw_tasks).await.map(|_| ())?;

    Ok(())
}

async fn get_index_status(db: &SurrealDbClient, index_name: &str, table: &str) -> Result<String> {
    let info_query = format!("INFO FOR INDEX {index_name} ON TABLE {table};");
    let mut info_res = db
        .client
        .query(info_query)
        .await
        .context("checking index status")?;
    let info: Option<Value> = info_res.take(0).context("failed to take info result")?;

    let Some(info) = info else {
        return Ok("unknown".to_string());
    };

    let parsed: IndexInfoForIndex =
        serde_json::from_value(info).context("deserializing INFO FOR INDEX response")?;

    Ok(parsed.building_status())
}

async fn rebuild_inner(db: &SurrealDbClient) -> Result<()> {
    debug!("Rebuilding indexes with concurrent definitions");
    create_fts_analyzer(db).await?;

    for spec in fts_index_specs() {
        if !index_exists(db, spec.table, spec.index_name).await? {
            debug!(
                index = spec.index_name,
                table = spec.table,
                "Skipping FTS rebuild because index is missing"
            );
            continue;
        }

        create_index_with_polling(
            db,
            spec.overwrite_definition(),
            spec.index_name,
            spec.table,
            Some(spec.table),
        )
        .await?;
    }

    let hnsw_tasks = hnsw_index_specs().into_iter().map(|spec| async move {
        if !index_exists(db, spec.table, spec.index_name).await? {
            debug!(
                index = spec.index_name,
                table = spec.table,
                "Skipping HNSW rebuild because index is missing"
            );
            return Ok(());
        }

        let Some(dimension) = existing_hnsw_dimension(db, &spec).await? else {
            warn!(
                index = spec.index_name,
                table = spec.table,
                "HNSW index missing dimension; skipping rebuild"
            );
            return Ok(());
        };

        create_index_with_polling(
            db,
            spec.definition_overwrite(dimension),
            spec.index_name,
            spec.table,
            Some(spec.table),
        )
        .await
    });

    try_join_all(hnsw_tasks).await.map(|_| ())
}

async fn existing_hnsw_dimension(
    db: &SurrealDbClient,
    spec: &HnswIndexSpec,
) -> Result<Option<usize>> {
    let Some(indexes) = table_index_definitions(db, spec.table).await? else {
        return Ok(None);
    };

    let Some(definition) = indexes
        .get(spec.index_name)
        .and_then(|details| details.get("Strand"))
        .and_then(|v| v.as_str())
    else {
        return Ok(None);
    };

    Ok(extract_dimension(definition).and_then(|d| usize::try_from(d).ok()))
}

async fn hnsw_index_state(
    db: &SurrealDbClient,
    spec: &HnswIndexSpec,
    expected_dimension: usize,
) -> Result<HnswIndexState> {
    match existing_hnsw_dimension(db, spec).await? {
        None => Ok(HnswIndexState::Missing),
        Some(current_dimension) if current_dimension == expected_dimension => {
            Ok(HnswIndexState::Matches)
        }
        Some(current_dimension) => Ok(HnswIndexState::Different(current_dimension as u64)),
    }
}

enum HnswIndexState {
    Missing,
    Matches,
    Different(u64),
}

fn extract_dimension(definition: &str) -> Option<u64> {
    definition
        .split("DIMENSION")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|token| token.trim_end_matches(';').parse::<u64>().ok())
}

async fn create_fts_analyzer(db: &SurrealDbClient) -> Result<()> {
    // Prefer snowball stemming when supported; fall back to ascii-only when the filter
    // is unavailable in the running Surreal build. Use IF NOT EXISTS to avoid clobbering
    // an existing analyzer definition.
    let snowball_query = format!(
        "DEFINE ANALYZER IF NOT EXISTS {FTS_ANALYZER_NAME}
            TOKENIZERS class
            FILTERS lowercase, ascii, snowball(english);"
    );

    match db.client.query(snowball_query).await {
        Ok(res) => {
            if res.check().is_ok() {
                return Ok(());
            }
            warn!(
                "Snowball analyzer check failed; attempting ascii fallback definition (analyzer: {})",
                FTS_ANALYZER_NAME
            );
        }
        Err(err) => {
            warn!(
                error = %err,
                "Snowball analyzer creation errored; attempting ascii fallback definition"
            );
        }
    }

    let fallback_query = format!(
        "DEFINE ANALYZER IF NOT EXISTS {FTS_ANALYZER_NAME}
            TOKENIZERS class
            FILTERS lowercase, ascii;"
    );

    let res = db
        .client
        .query(fallback_query)
        .await
        .context("creating fallback FTS analyzer")?;

    if let Err(err) = res.check() {
        warn!(
            error = %err,
            "Fallback analyzer creation failed; FTS will run without snowball/ascii analyzer ({})",
            FTS_ANALYZER_NAME
        );
        return Err(err).context("failed to create fallback FTS analyzer");
    }

    warn!(
        "Snowball analyzer unavailable; using fallback analyzer ({}) with lowercase+ascii only",
        FTS_ANALYZER_NAME
    );

    Ok(())
}

async fn create_index_with_polling(
    db: &SurrealDbClient,
    definition: String,
    index_name: &str,
    table: &str,
    progress_table: Option<&str>,
) -> Result<()> {
    const MAX_ATTEMPTS: usize = 3;
    let expected_total = match progress_table {
        Some(table) => Some(count_table_rows(db, table).await.with_context(|| {
            format!("counting rows in {table} for index {index_name} progress")
        })?),
        None => None,
    };

    let mut attempts: usize = 0;
    loop {
        attempts = attempts.saturating_add(1);
        let res = db
            .client
            .query(definition.clone())
            .await
            .with_context(|| format!("creating index {index_name} on table {table}"))?;
        match res.check() {
            Ok(_) => break,
            Err(err) => {
                let msg = err.to_string();
                let conflict = msg.contains("read or write conflict");
                warn!(
                    index = %index_name,
                    table = %table,
                    error = ?err,
                    attempt = attempts,
                    definition = %definition,
                    "Index definition failed"
                );
                if conflict && attempts < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                return Err(err).with_context(|| {
                    format!("index definition failed for {index_name} on {table}")
                });
            }
        }
    }

    debug!(
        index = %index_name,
        table = %table,
        expected_rows = ?expected_total,
        "Index definition submitted; waiting for build to finish"
    );

    poll_index_build_status(db, index_name, table, expected_total, INDEX_POLL_INTERVAL).await
}

async fn poll_index_build_status(
    db: &SurrealDbClient,
    index_name: &str,
    table: &str,
    total_rows: Option<u64>,
    poll_every: Duration,
) -> Result<()> {
    let started_at = std::time::Instant::now();
    let mut last_snapshot: Option<IndexBuildSnapshot> = None;

    loop {
        if started_at.elapsed() >= INDEX_BUILD_TIMEOUT {
            return Err(anyhow::anyhow!(
                "index build timed out after {:?} for {index_name} on {table} (last status: {})",
                INDEX_BUILD_TIMEOUT,
                last_snapshot
                    .as_ref()
                    .map_or("unknown", |snapshot| snapshot.status.as_str())
            ))
            .with_context(|| format!("index {index_name} on table {table} did not become ready"));
        }

        tokio::time::sleep(poll_every).await;

        let info_query = format!("INFO FOR INDEX {index_name} ON TABLE {table};");
        let mut info_res =
            db.client.query(info_query).await.with_context(|| {
                format!("checking index build status for {index_name} on {table}")
            })?;

        let info: Option<Value> = info_res
            .take(0)
            .context("failed to deserialize INFO FOR INDEX result")?;

        let Some(snapshot) = parse_index_build_info(info, total_rows) else {
            return Err(anyhow::anyhow!(
                "INFO FOR INDEX returned no data for {index_name} on {table}"
            ));
        };

        last_snapshot = Some(snapshot.clone());

        if let Some(pct) = snapshot.progress_pct {
            debug!(
                index = %index_name,
                table = %table,
                status = snapshot.status,
                initial = snapshot.initial,
                pending = snapshot.pending,
                updated = snapshot.updated,
                processed = snapshot.processed,
                total = snapshot.total_rows,
                progress_pct = format_args!("{pct:.1}"),
                "Index build status"
            );
        } else {
            debug!(
                index = %index_name,
                table = %table,
                status = snapshot.status,
                initial = snapshot.initial,
                pending = snapshot.pending,
                updated = snapshot.updated,
                processed = snapshot.processed,
                "Index build status"
            );
        }

        if snapshot.is_ready() {
            debug!(
                index = %index_name,
                table = %table,
                elapsed = ?started_at.elapsed(),
                processed = snapshot.processed,
                total = snapshot.total_rows,
                "Index is ready"
            );
            return Ok(());
        }

        if snapshot.status.eq_ignore_ascii_case("error") {
            return Err(anyhow::anyhow!(
                "index build failed for {index_name} on {table}: status=error, processed={}, total={:?}",
                snapshot.processed,
                snapshot.total_rows
            ));
        }
    }
}

/// `building` block from SurrealDB `INFO FOR INDEX` (concurrent index builds).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct IndexBuildingProgress {
    #[serde(default)]
    initial: u64,
    #[serde(default)]
    pending: u64,
    #[serde(default)]
    updated: u64,
    #[serde(default)]
    status: String,
}

/// Top-level `INFO FOR INDEX` payload shape (SurrealDB v2.x).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
struct IndexInfoForIndex {
    #[serde(default)]
    building: Option<IndexBuildingProgress>,
}

impl IndexInfoForIndex {
    fn building_status(&self) -> String {
        match &self.building {
            None => "ready".to_string(),
            Some(progress) if progress.status.is_empty() => "ready".to_string(),
            Some(progress) => progress.status.clone(),
        }
    }

    fn into_build_snapshot(self, total_rows: Option<u64>) -> IndexBuildSnapshot {
        let (initial, pending, updated, status) = match self.building {
            None => (0, 0, 0, "ready".to_string()),
            Some(progress) => {
                let status = if progress.status.is_empty() {
                    "ready".to_string()
                } else {
                    progress.status
                };
                (progress.initial, progress.pending, progress.updated, status)
            }
        };

        let processed = initial.saturating_add(updated);
        let progress_pct = total_rows.map(|total| {
            if total == 0 {
                0.0
            } else {
                ((f64::from(u32::try_from(processed).unwrap_or(u32::MAX))
                    / f64::from(u32::try_from(total).unwrap_or(1)))
                .min(1.0))
                    * 100.0
            }
        });

        IndexBuildSnapshot {
            status,
            initial,
            pending,
            updated,
            processed,
            total_rows,
            progress_pct,
        }
    }
}

/// Snapshot of an index build progress as reported by SurrealDB's `INFO FOR INDEX`.
#[derive(Debug, Clone, PartialEq)]
struct IndexBuildSnapshot {
    /// Current build status string (e.g., `"indexing"`, `"ready"`, `"error"`).
    status: String,
    /// Number of rows present when the build started.
    initial: u64,
    /// Number of rows still pending processing.
    pending: u64,
    /// Number of rows updated since the build started.
    updated: u64,
    /// Total rows processed so far (`initial + updated`).
    processed: u64,
    /// Total rows expected (from `SELECT count()` before the build), if available.
    total_rows: Option<u64>,
    /// Progress as a percentage of `processed / total_rows`, if `total_rows` is known.
    progress_pct: Option<f64>,
}

impl IndexBuildSnapshot {
    fn is_ready(&self) -> bool {
        self.status.eq_ignore_ascii_case("ready")
    }
}

fn parse_index_build_info(
    info: Option<Value>,
    total_rows: Option<u64>,
) -> Option<IndexBuildSnapshot> {
    let info = info?;
    let parsed: IndexInfoForIndex = serde_json::from_value(info).ok()?;
    Some(parsed.into_build_snapshot(total_rows))
}

#[derive(Debug, Deserialize)]
struct CountRow {
    count: u64,
}

async fn count_table_rows(db: &SurrealDbClient, table: &str) -> Result<u64> {
    let query = format!("SELECT count() AS count FROM {table} GROUP ALL;");
    let mut response = db
        .client
        .query(query)
        .await
        .with_context(|| format!("counting rows in {table}"))?;
    let rows: Vec<CountRow> = response
        .take(0)
        .context("failed to deserialize count() response")?;
    Ok(rows.first().map_or(0, |r| r.count))
}

async fn table_index_definitions(
    db: &SurrealDbClient,
    table: &str,
) -> Result<Option<Map<String, Value>>> {
    let info_query = format!("INFO FOR TABLE {table};");
    let mut response = db
        .client
        .query(info_query)
        .await
        .with_context(|| format!("fetching table info for {table}"))?;

    let info: surrealdb::Value = response
        .take(0)
        .context("failed to take table info response")?;

    let info_json: Value =
        serde_json::to_value(info).context("serializing table info to JSON for parsing")?;

    Ok(info_json
        .get("Object")
        .and_then(|o| o.get("indexes"))
        .and_then(|i| i.get("Object"))
        .and_then(|i| i.as_object())
        .cloned())
}

async fn index_exists(db: &SurrealDbClient, table: &str, index_name: &str) -> Result<bool> {
    let Some(indexes) = table_index_definitions(db, table).await? else {
        return Ok(false);
    };

    Ok(indexes.contains_key(index_name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::must_use_candidate)]
    use crate::storage::db::SurrealDbClient;
    use anyhow::{self, Context};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn parse_index_build_info_reports_progress() -> anyhow::Result<()> {
        let info = json!({
            "building": {
                "initial": 56894,
                "pending": 0,
                "status": "indexing",
                "updated": 0
            }
        });

        let snapshot = parse_index_build_info(Some(info), Some(61081)).context("snapshot")?;
        assert_eq!(
            snapshot,
            IndexBuildSnapshot {
                status: "indexing".to_string(),
                initial: 56894,
                pending: 0,
                updated: 0,
                processed: 56894,
                total_rows: Some(61081),
                progress_pct: Some((56894_f64 / 61081_f64) * 100.0),
            }
        );
        assert!(!snapshot.is_ready());
        Ok(())
    }

    #[test]
    fn parse_index_build_info_defaults_to_ready_when_no_building_block() -> anyhow::Result<()> {
        // Surreal returns `{}` when the index exists but isn't building.
        let info = json!({});
        let snapshot = parse_index_build_info(Some(info), Some(10)).context("snapshot")?;
        assert!(snapshot.is_ready());
        assert_eq!(snapshot.processed, 0);
        assert_eq!(snapshot.progress_pct, Some(0.0));
        Ok(())
    }

    #[test]
    fn index_info_for_index_deserializes_ready_status_shape() -> anyhow::Result<()> {
        let info = json!({
            "building": {
                "status": "ready"
            }
        });

        let parsed: IndexInfoForIndex =
            serde_json::from_value(info).context("deserialize ready shape")?;
        assert_eq!(parsed.building_status(), "ready");

        let snapshot = parse_index_build_info(
            Some(json!({
                "building": { "status": "ready" }
            })),
            None,
        )
        .context("snapshot")?;
        assert!(snapshot.is_ready());
        assert_eq!(snapshot.initial, 0);
        Ok(())
    }

    #[test]
    fn index_info_for_index_deserializes_indexing_shape_from_surreal_docs() -> anyhow::Result<()> {
        let info = json!({
            "building": {
                "initial": 8143,
                "pending": 19,
                "status": "indexing",
                "updated": 80
            }
        });

        let parsed: IndexInfoForIndex =
            serde_json::from_value(info.clone()).context("deserialize indexing shape")?;
        assert_eq!(parsed.building_status(), "indexing");

        let snapshot = parse_index_build_info(Some(info), None).context("snapshot")?;
        assert_eq!(snapshot.status, "indexing");
        assert_eq!(snapshot.initial, 8143);
        assert_eq!(snapshot.pending, 19);
        assert_eq!(snapshot.updated, 80);
        assert_eq!(snapshot.processed, 8223);
        assert!(!snapshot.is_ready());
        Ok(())
    }

    #[test]
    fn parse_index_build_info_reports_error_status() -> anyhow::Result<()> {
        let info = json!({
            "building": {
                "initial": 100,
                "pending": 5,
                "status": "error",
                "updated": 10
            }
        });

        let snapshot = parse_index_build_info(Some(info), Some(200)).context("snapshot")?;
        assert_eq!(snapshot.status, "error");
        assert!(!snapshot.is_ready());
        Ok(())
    }

    #[test]
    fn extract_dimension_parses_value() {
        let definition = "DEFINE INDEX idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32 EFC 100 M 8;";
        assert_eq!(extract_dimension(definition), Some(1536));
    }

    #[tokio::test]
    async fn ensure_runtime_is_idempotent() -> anyhow::Result<()> {
        let namespace = "indexes_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .context("in-memory db")?;

        db.apply_migrations()
            .await
            .context("migrations should succeed")?;

        ensure_runtime(&db, 1536)
            .await
            .context("first call should succeed")?;
        ensure_runtime(&db, 1536)
            .await
            .context("second index creation")?;
        Ok(())
    }

    #[tokio::test]
    async fn ensure_hnsw_index_overwrites_dimension() -> anyhow::Result<()> {
        let namespace = "indexes_dim";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .context("in-memory db")?;

        db.apply_migrations()
            .await
            .context("migrations should succeed")?;

        ensure_runtime(&db, 1536)
            .await
            .context("initial index creation")?;
        ensure_runtime(&db, 128)
            .await
            .context("overwritten index creation")?;
        Ok(())
    }
}
