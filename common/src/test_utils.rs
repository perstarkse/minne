//! Shared helpers for in-memory SurrealDB tests.
#![cfg(any(test, feature = "test-utils"))]

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::storage::{
    db::SurrealDbClient,
    indexes::{ensure_runtime, rebuild},
    types::{
        EmbeddingRecord, knowledge_entity_embedding::KnowledgeEntityEmbedding,
        system_settings::SystemSettings, text_chunk_embedding::TextChunkEmbedding,
    },
};

const TEST_NAMESPACE: &str = "test_ns";

/// Starts an in-memory database, applies migrations, and returns a client.
///
/// # Errors
///
/// Returns an error if the database cannot be started or migrations fail.
pub async fn setup_test_db() -> Result<SurrealDbClient> {
    let database = Uuid::new_v4().to_string();
    let db = SurrealDbClient::memory(TEST_NAMESPACE, &database)
        .await
        .context("start in-memory surrealdb")?;

    db.apply_migrations().await.context("apply migrations")?;

    Ok(db)
}

/// Updates singleton [`SystemSettings`] embedding dimensions for tests.
///
/// # Errors
///
/// Returns an error if settings cannot be loaded or updated.
pub async fn configure_embedding_dimension(db: &SurrealDbClient, dimension: u32) -> Result<()> {
    let mut settings = SystemSettings::get_current(db).await?;
    settings.embedding_dimensions = dimension;
    SystemSettings::update(db, settings).await?;
    Ok(())
}

/// Starts a test database and sets the embedding dimension in system settings.
///
/// # Errors
///
/// Returns an error if setup or settings update fails.
pub async fn setup_test_db_with_embedding_dimension(dimension: u32) -> Result<SurrealDbClient> {
    let db = setup_test_db().await?;
    configure_embedding_dimension(&db, dimension).await?;
    Ok(db)
}

/// Prepares a database for text-chunk embedding tests at the given dimension.
///
/// # Errors
///
/// Returns an error if setup, settings update, or index redefinition fails.
pub async fn prepare_text_chunk_test_db(dimension: u32) -> Result<SurrealDbClient> {
    let db = setup_test_db_with_embedding_dimension(dimension).await?;
    TextChunkEmbedding::redefine_hnsw_index(&db, dimension as usize)
        .await
        .with_context(|| format!("set text chunk index dimension to {dimension}"))?;
    Ok(db)
}

/// Prepares a database for knowledge-entity embedding tests at the given dimension.
///
/// # Errors
///
/// Returns an error if setup, settings update, or index redefinition fails.
pub async fn prepare_knowledge_entity_test_db(dimension: u32) -> Result<SurrealDbClient> {
    let db = setup_test_db_with_embedding_dimension(dimension).await?;
    KnowledgeEntityEmbedding::redefine_hnsw_index(&db, dimension as usize)
        .await
        .with_context(|| format!("set knowledge entity index dimension to {dimension}"))?;
    Ok(db)
}

/// Starts a test database and ensures runtime FTS/HNSW indexes are ready.
///
/// # Errors
///
/// Returns an error if setup, index creation, or rebuild fails.
pub async fn setup_test_db_with_runtime_indexes() -> Result<SurrealDbClient> {
    let db = setup_test_db().await?;
    ensure_runtime(&db, 1536).await?;
    rebuild(&db).await?;
    Ok(db)
}

/// Ensures an FTS analyzer and BM25 indexes exist for a table.
///
/// Attempts snowball(english) tokenizer first; falls back to basic
/// lowercase+ascii when the platform lacks the snowball extension.
///
/// `indexes` is a slice of `(field_name, index_id_suffix)` pairs —
/// e.g. `&[("chunk", "chunk")]` produces index
/// `text_chunk_fts_chunk_idx` on column `chunk` of `text_chunk`.
///
/// # Errors
///
/// Returns an error if the fallback definition fails. The initial
/// snowball attempt is allowed to fail silently.
pub async fn ensure_fts_index(
    db: &SurrealDbClient,
    table: &str,
    indexes: &[(&str, &str)],
) -> Result<()> {
    use std::fmt::Write;

    let mut define_indexes = String::new();
    for (field, suffix) in indexes {
        let _ = writeln!(
            define_indexes,
            "DEFINE INDEX IF NOT EXISTS {table}_fts_{suffix}_idx ON TABLE {table} FIELDS {field} SEARCH ANALYZER app_en_fts_analyzer BM25;"
        );
    }

    let snowball_sql = format!(
        "DEFINE ANALYZER IF NOT EXISTS app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii, snowball(english);\n{define_indexes}"
    );

    if let Err(err) = db.client.query(&snowball_sql).await {
        let fallback_sql = format!(
            "DEFINE ANALYZER OVERWRITE app_en_fts_analyzer TOKENIZERS class, punct FILTERS lowercase, ascii;\n{define_indexes}"
        );
        db.client
            .query(&fallback_sql)
            .await
            .with_context(|| format!("define fts index fallback for {table}: {err}"))?;
    }
    Ok(())
}
