use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

use crate::{error::AppError, storage::db::SurrealDbClient};

const INDEX_POLL_INTERVAL: Duration = Duration::from_secs(2);
const FTS_ANALYZER_NAME: &str = "app_en_fts_analyzer";

#[derive(Clone, Copy)]
struct HnswIndexSpec {
    index_name: &'static str,
    table: &'static str,
    options: &'static str,
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
}

/// Build runtime Surreal indexes (FTS + HNSW) using concurrent creation with readiness polling.
/// Idempotent: safe to call multiple times and will overwrite HNSW definitions when the dimension changes.
pub async fn ensure_runtime_indexes(
    db: &SurrealDbClient,
    embedding_dimension: usize,
) -> Result<(), AppError> {
    ensure_runtime_indexes_inner(db, embedding_dimension)
        .await
        .map_err(|err| AppError::InternalError(err.to_string()))
}

async fn ensure_runtime_indexes_inner(
    db: &SurrealDbClient,
    embedding_dimension: usize,
) -> Result<()> {
    create_fts_analyzer(db).await?;

    for spec in fts_index_specs() {
        create_index_with_polling(
            db,
            spec.definition(),
            spec.index_name,
            spec.table,
            Some(spec.table),
        )
        .await?;
    }

    for spec in hnsw_index_specs() {
        ensure_hnsw_index(db, &spec, embedding_dimension).await?;
    }

    Ok(())
}

async fn ensure_hnsw_index(
    db: &SurrealDbClient,
    spec: &HnswIndexSpec,
    dimension: usize,
) -> Result<()> {
    let definition = match hnsw_index_state(db, spec, dimension).await? {
        HnswIndexState::Missing => spec.definition_if_not_exists(dimension),
        HnswIndexState::Matches(_) => spec.definition_if_not_exists(dimension),
        HnswIndexState::Different(existing) => {
            info!(
                index = spec.index_name,
                table = spec.table,
                existing_dimension = existing,
                target_dimension = dimension,
                "Overwriting HNSW index to match new embedding dimension"
            );
            spec.definition_overwrite(dimension)
        }
    };

    create_index_with_polling(
        db,
        definition,
        spec.index_name,
        spec.table,
        Some(spec.table),
    )
    .await
}

async fn hnsw_index_state(
    db: &SurrealDbClient,
    spec: &HnswIndexSpec,
    expected_dimension: usize,
) -> Result<HnswIndexState> {
    let info_query = format!("INFO FOR TABLE {table};", table = spec.table);
    let mut response = db
        .client
        .query(info_query)
        .await
        .with_context(|| format!("fetching table info for {}", spec.table))?;

    let info: surrealdb::Value = response
        .take(0)
        .context("failed to take table info response")?;

    let info_json: Value =
        serde_json::to_value(info).context("serializing table info to JSON for parsing")?;

    let Some(indexes) = info_json
        .get("Object")
        .and_then(|o| o.get("indexes"))
        .and_then(|i| i.get("Object"))
        .and_then(|i| i.as_object())
    else {
        return Ok(HnswIndexState::Missing);
    };

    let Some(definition) = indexes
        .get(spec.index_name)
        .and_then(|details| details.get("Strand"))
        .and_then(|v| v.as_str())
    else {
        return Ok(HnswIndexState::Missing);
    };

    let Some(current_dimension) = extract_dimension(definition) else {
        return Ok(HnswIndexState::Missing);
    };

    if current_dimension == expected_dimension as u64 {
        Ok(HnswIndexState::Matches(current_dimension))
    } else {
        Ok(HnswIndexState::Different(current_dimension))
    }
}

enum HnswIndexState {
    Missing,
    Matches(u64),
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
    let analyzer_query = format!(
        "DEFINE ANALYZER IF NOT EXISTS {analyzer}
            TOKENIZERS class
            FILTERS lowercase, ascii, snowball(english);",
        analyzer = FTS_ANALYZER_NAME
    );

    let res = db
        .client
        .query(analyzer_query)
        .await
        .context("creating FTS analyzer")?;

    res.check().context("failed to create FTS analyzer")?;
    Ok(())
}

async fn create_index_with_polling(
    db: &SurrealDbClient,
    definition: String,
    index_name: &str,
    table: &str,
    progress_table: Option<&str>,
) -> Result<()> {
    let expected_total = match progress_table {
        Some(table) => Some(count_table_rows(db, table).await.with_context(|| {
            format!("counting rows in {table} for index {index_name} progress")
        })?),
        None => None,
    };

    let res = db
        .client
        .query(definition)
        .await
        .with_context(|| format!("creating index {index_name} on table {table}"))?;
    res.check()
        .with_context(|| format!("index definition failed for {index_name} on {table}"))?;

    info!(
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

    loop {
        tokio::time::sleep(poll_every).await;

        let info_query = format!("INFO FOR INDEX {index_name} ON TABLE {table};");
        let mut info_res = db.client.query(info_query).await.with_context(|| {
            format!("checking index build status for {index_name} on {table}")
        })?;

        let info: Option<Value> = info_res
            .take(0)
            .context("failed to deserialize INFO FOR INDEX result")?;

        let Some(snapshot) = parse_index_build_info(info, total_rows) else {
            warn!(
                index = %index_name,
                table = %table,
                "INFO FOR INDEX returned no data; assuming index definition might be missing"
            );
            break;
        };

        match snapshot.progress_pct {
            Some(pct) => info!(
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
            ),
            None => info!(
                index = %index_name,
                table = %table,
                status = snapshot.status,
                initial = snapshot.initial,
                pending = snapshot.pending,
                updated = snapshot.updated,
                processed = snapshot.processed,
                "Index build status"
            ),
        }

        if snapshot.is_ready() {
            info!(
                index = %index_name,
                table = %table,
                elapsed = ?started_at.elapsed(),
                processed = snapshot.processed,
                total = snapshot.total_rows,
                "Index is ready"
            );
            break;
        }

        if snapshot.status.eq_ignore_ascii_case("error") {
            warn!(
                index = %index_name,
                table = %table,
                status = snapshot.status,
                "Index build reported error status; stopping polling"
            );
            break;
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq)]
struct IndexBuildSnapshot {
    status: String,
    initial: u64,
    pending: u64,
    updated: u64,
    processed: u64,
    total_rows: Option<u64>,
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
    let building = info.get("building");

    let status = building
        .and_then(|b| b.get("status"))
        .and_then(|s| s.as_str())
        // If there's no `building` block at all, treat as "ready" (index not building anymore)
        .unwrap_or("ready")
        .to_string();

    let initial = building
        .and_then(|b| b.get("initial"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let pending = building
        .and_then(|b| b.get("pending"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let updated = building
        .and_then(|b| b.get("updated"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // `initial` is the number of rows seen when the build started; `updated` accounts for later writes.
    let processed = initial.saturating_add(updated);

    let progress_pct = total_rows.map(|total| {
        if total == 0 {
            0.0
        } else {
            ((processed as f64 / total as f64).min(1.0)) * 100.0
        }
    });

    Some(IndexBuildSnapshot {
        status,
        initial,
        pending,
        updated,
        processed,
        total_rows,
        progress_pct,
    })
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
    Ok(rows.first().map(|r| r.count).unwrap_or(0))
}

const fn hnsw_index_specs() -> [HnswIndexSpec; 2] {
    [
        HnswIndexSpec {
            index_name: "idx_embedding_text_chunk_embedding",
            table: "text_chunk_embedding",
            options: "DIST COSINE TYPE F32 EFC 100 M 8 CONCURRENTLY",
        },
        HnswIndexSpec {
            index_name: "idx_embedding_knowledge_entity_embedding",
            table: "knowledge_entity_embedding",
            options: "DIST COSINE TYPE F32 EFC 100 M 8 CONCURRENTLY",
        },
    ]
}

const fn fts_index_specs() -> [FtsIndexSpec; 9] {
    [
        FtsIndexSpec {
            index_name: "text_content_fts_idx",
            table: "text_content",
            field: "text",
            analyzer: Some(FTS_ANALYZER_NAME),
            method: "BM25",
        },
        FtsIndexSpec {
            index_name: "text_content_category_fts_idx",
            table: "text_content",
            field: "category",
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn parse_index_build_info_reports_progress() {
        let info = json!({
            "building": {
                "initial": 56894,
                "pending": 0,
                "status": "indexing",
                "updated": 0
            }
        });

        let snapshot = parse_index_build_info(Some(info), Some(61081)).expect("snapshot");
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
    }

    #[test]
    fn parse_index_build_info_defaults_to_ready_when_no_building_block() {
        // Surreal returns `{}` when the index exists but isn't building.
        let info = json!({});
        let snapshot = parse_index_build_info(Some(info), Some(10)).expect("snapshot");
        assert!(snapshot.is_ready());
        assert_eq!(snapshot.processed, 0);
        assert_eq!(snapshot.progress_pct, Some(0.0));
    }

    #[test]
    fn extract_dimension_parses_value() {
        let definition = "DEFINE INDEX idx_embedding_text_chunk_embedding ON TABLE text_chunk_embedding FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32 EFC 100 M 8;";
        assert_eq!(extract_dimension(definition), Some(1536));
    }

    #[tokio::test]
    async fn ensure_runtime_indexes_is_idempotent() {
        let namespace = "indexes_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("in-memory db");

        db.apply_migrations()
            .await
            .expect("migrations should succeed");

        // First run creates everything
        ensure_runtime_indexes(&db, 1536)
            .await
            .expect("initial index creation");

        // Second run should be a no-op and still succeed
        ensure_runtime_indexes(&db, 1536)
            .await
            .expect("second index creation");
    }

    #[tokio::test]
    async fn ensure_hnsw_index_overwrites_dimension() {
        let namespace = "indexes_dim";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("in-memory db");

        db.apply_migrations()
            .await
            .expect("migrations should succeed");

        // Create initial index with default dimension
        ensure_runtime_indexes(&db, 1536)
            .await
            .expect("initial index creation");

        // Change dimension and ensure overwrite path is exercised
        ensure_runtime_indexes(&db, 128)
            .await
            .expect("overwritten index creation");
    }
}
