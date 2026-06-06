use anyhow::Context;
use common::{
    storage::{
        db::SurrealDbClient,
        indexes::{embedding_index_dimension, ensure_runtime},
        types::{
            knowledge_entity::KnowledgeEntity, system_settings::SystemSettings,
            text_chunk::TextChunk,
        },
    },
    utils::embedding::EmbeddingProvider,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use super::SharedServices;

/// How a process participates in embedding-runtime maintenance.
///
/// Embedding configuration changes (model/dimension) take effect on restart: the active
/// [`EmbeddingProvider`] is built once at startup, so the stored vectors must be reconciled to it
/// before indexes are rebuilt. Only a single maintainer should perform that (potentially long,
/// destructive) re-embed; query-only servers stay read-only to avoid racing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Each binary (main/worker/server) constructs only one variant, so the other looks dead within
// that single compilation unit even though both are used across the binary set.
#[allow(dead_code)]
pub enum EmbeddingRuntimeRole {
    /// Combined binary or worker: re-embeds stored data when it no longer matches the provider.
    Maintainer,
    /// Server-only: never mutates stored embeddings; aligns indexes to the data that exists.
    ReadOnly,
}

/// Re-embed lock TTL. Generously sized so a slow re-embed of a large corpus never expires
/// out from under the maintainer that holds it; an abandoned lock (crashed maintainer) self-heals.
const REEMBED_LOCK_TTL: &str = "30m";

/// Reconciles embeddings with the active provider and ensures runtime indexes are ready.
///
/// Detection is based on the stored chunk-embedding HNSW index dimension (a persisted marker of
/// the embedding space actually in the database). When it differs from the active provider's
/// dimension, a [`EmbeddingRuntimeRole::Maintainer`] re-embeds before indexes are (re)built;
/// a [`EmbeddingRuntimeRole::ReadOnly`] server leaves indexes aligned to the existing data and
/// serves in a degraded state until a maintainer reconciles.
///
/// # Errors
///
/// Returns an error if syncing settings, inspecting/building indexes, or re-embedding fails.
pub async fn prepare_embedding_runtime(
    services: &SharedServices,
    role: EmbeddingRuntimeRole,
) -> anyhow::Result<SystemSettings> {
    // Keep SystemSettings in sync with the active provider so the admin UI reflects the real
    // backend/model/dimension. This does not, by itself, decide whether a re-embed is needed.
    let (settings, _changed) =
        SystemSettings::sync_from_embedding_provider(&services.db, &services.embedding_provider)
            .await
            .context("sync system settings from embedding provider")?;

    let target_dim = services.embedding_provider.dimension();
    let stored_dim = embedding_index_dimension(&services.db)
        .await
        .context("inspect stored embedding index dimension")?;
    let mismatch = matches!(stored_dim, Some(dim) if dim != target_dim);

    let index_dim = if mismatch {
        match role {
            EmbeddingRuntimeRole::Maintainer => {
                reconcile_embeddings(&services.db, &services.embedding_provider, target_dim)
                    .await?;
                target_dim
            }
            EmbeddingRuntimeRole::ReadOnly => {
                warn!(
                    stored_dimension = stored_dim,
                    target_dimension = target_dim,
                    "Stored embeddings do not match the active embedding dimension. A maintainer \
                     (worker) must re-embed; serving in a degraded state and keeping indexes \
                     aligned to the existing data until then."
                );
                // Preserve the index that matches the vectors actually stored. Do not overwrite it
                // to the new dimension here — that would happen before the data is re-embedded and
                // would break retrieval entirely.
                stored_dim.unwrap_or(target_dim)
            }
        }
    } else {
        target_dim
    };

    ensure_runtime(&services.db, index_dim)
        .await
        .context("ensure runtime indexes")?;

    Ok(settings)
}

/// Acquires the re-embed lock (so only one maintainer reconciles), re-embeds, then releases it.
async fn reconcile_embeddings(
    db: &SurrealDbClient,
    embedding_provider: &EmbeddingProvider,
    target_dim: usize,
) -> anyhow::Result<()> {
    let owner = reembed_lock_owner();

    if !try_acquire_reembed_lock(db, &owner).await? {
        info!("Another maintainer holds the re-embed lock; skipping re-embed on this instance");
        return Ok(());
    }

    let result = reconcile_under_lock(db, embedding_provider, target_dim).await;
    release_reembed_lock(db, &owner).await;
    result
}

/// Re-embed body executed while holding the lock, with a re-check to avoid duplicate work.
async fn reconcile_under_lock(
    db: &SurrealDbClient,
    embedding_provider: &EmbeddingProvider,
    target_dim: usize,
) -> anyhow::Result<()> {
    // A peer may have finished re-embedding between detection and lock acquisition.
    let stored_dim = embedding_index_dimension(db)
        .await
        .context("re-check stored embedding dimension under lock")?;
    if !matches!(stored_dim, Some(dim) if dim != target_dim) {
        info!("Stored embeddings already match the active dimension; skipping re-embed");
        return Ok(());
    }

    let target_dim_u32 = u32::try_from(target_dim)
        .map_err(|_| anyhow::anyhow!("embedding dimension {target_dim} exceeds u32::MAX"))?;
    re_embed_all(db, embedding_provider, target_dim_u32).await
}

async fn re_embed_all(
    db: &SurrealDbClient,
    embedding_provider: &EmbeddingProvider,
    embedding_dimensions: u32,
) -> anyhow::Result<()> {
    warn!(
        embedding_dimensions,
        "Embedding configuration changed; re-embedding existing data"
    );

    info!("Re-embedding TextChunks");
    TextChunk::update_all_embeddings(db, embedding_provider)
        .await
        .context("re-embed text chunks after embedding dimension change")?;

    info!("Re-embedding KnowledgeEntities");
    KnowledgeEntity::update_all_embeddings(db, embedding_provider)
        .await
        .context("re-embed knowledge entities after embedding dimension change")?;

    info!("Re-embedding complete");
    Ok(())
}

/// A process-unique token identifying this re-embed lock acquisition (for release).
fn reembed_lock_owner() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("reembed-{}-{nanos}", std::process::id())
}

/// Best-effort atomic mutex over the (potentially long) re-embed using a singleton record.
///
/// `CREATE` of a fixed record id fails if it already exists, which serializes concurrent
/// maintainers. An expired lock is reaped first so a crashed maintainer cannot block forever.
async fn try_acquire_reembed_lock(db: &SurrealDbClient, owner: &str) -> anyhow::Result<bool> {
    db.client
        .query("DEFINE TABLE IF NOT EXISTS maintenance_lock SCHEMALESS;")
        .await
        .and_then(surrealdb::Response::check)
        .context("define maintenance_lock table")?;

    db.client
        .query("DELETE maintenance_lock:reembed WHERE expires_at < time::now();")
        .await
        .and_then(surrealdb::Response::check)
        .context("reap expired re-embed lock")?;

    // `CREATE` of a fixed record id succeeds for the first caller and errors with an
    // "already exists" record conflict for any concurrent caller, giving us an atomic mutex.
    let acquired = db
        .client
        .query(format!(
            "CREATE maintenance_lock:reembed SET owner = $owner, expires_at = time::now() + {REEMBED_LOCK_TTL};"
        ))
        .bind(("owner", owner.to_string()))
        .await
        .and_then(surrealdb::Response::check)
        .is_ok();

    Ok(acquired)
}

async fn release_reembed_lock(db: &SurrealDbClient, owner: &str) {
    let released = db
        .client
        .query("DELETE maintenance_lock:reembed WHERE owner = $owner;")
        .bind(("owner", owner.to_string()))
        .await
        .and_then(surrealdb::Response::check);

    if let Err(err) = released {
        warn!(error = %err, "Failed to release re-embed lock; it will expire automatically");
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use common::{
        storage::{
            db::SurrealDbClient,
            indexes::{embedding_index_dimension, ensure_runtime},
            types::{system_settings::SystemSettings, text_chunk::TextChunk},
        },
        utils::embedding::EmbeddingProvider,
    };

    use crate::bootstrap::tests::init_smoke_services;

    async fn test_db() -> SurrealDbClient {
        SurrealDbClient::memory("reembed_lock_ns", &reembed_lock_owner())
            .await
            .expect("in-memory db")
    }

    /// Index at `stored_dim`, active provider at `target_dim` (no chunks — re-embed only rebuilds indexes).
    async fn services_with_index_provider_mismatch(
        stored_dim: usize,
        target_dim: usize,
    ) -> (super::SharedServices, std::path::PathBuf) {
        let (mut services, data_dir) = init_smoke_services().await.expect("smoke services");

        ensure_runtime(&services.db, stored_dim)
            .await
            .expect("seed index at stored dimension");

        let mut settings = SystemSettings::get_current(&services.db)
            .await
            .expect("settings");
        settings.embedding_dimensions = u32::try_from(target_dim).expect("target dim fits u32");
        SystemSettings::update(&services.db, settings)
            .await
            .expect("update settings");

        services.embedding_provider =
            Arc::new(EmbeddingProvider::new_hashed(target_dim).expect("hashed provider for test"));

        (services, data_dir)
    }

    #[tokio::test]
    async fn maintainer_reconciles_index_when_provider_dimension_differs() {
        let (services, data_dir) = services_with_index_provider_mismatch(3, 5).await;

        prepare_embedding_runtime(&services, EmbeddingRuntimeRole::Maintainer)
            .await
            .expect("maintainer startup");

        assert_eq!(
            embedding_index_dimension(&services.db)
                .await
                .expect("index dim"),
            Some(5),
            "maintainer should rebuild the index to the provider dimension"
        );

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }

    #[tokio::test]
    async fn read_only_startup_preserves_index_when_provider_dimension_differs() {
        let (services, data_dir) = services_with_index_provider_mismatch(3, 5).await;

        prepare_embedding_runtime(&services, EmbeddingRuntimeRole::ReadOnly)
            .await
            .expect("read-only startup");

        assert_eq!(
            embedding_index_dimension(&services.db)
                .await
                .expect("index dim"),
            Some(3),
            "read-only server must not overwrite the index before a maintainer re-embeds"
        );

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }

    #[tokio::test]
    async fn maintainer_reembeds_chunks_when_index_dimension_differs() {
        let (mut services, data_dir) = init_smoke_services().await.expect("smoke services");

        let mut settings = SystemSettings::get_current(&services.db)
            .await
            .expect("settings");
        settings.embedding_dimensions = 3;
        SystemSettings::update(&services.db, settings)
            .await
            .expect("settings at stored dimension");
        services.embedding_provider =
            Arc::new(EmbeddingProvider::new_hashed(3).expect("stored-dimension provider"));

        ensure_runtime(&services.db, 3)
            .await
            .expect("seed index at stored dimension");

        let chunk = TextChunk::new(
            "reembed-src".into(),
            "dimension migration test chunk".into(),
            "user1".into(),
        );
        TextChunk::store_with_embedding(chunk, vec![0.1, 0.2, 0.3], &services.db)
            .await
            .expect("store chunk at old dimension");

        let mut settings = SystemSettings::get_current(&services.db)
            .await
            .expect("settings");
        settings.embedding_dimensions = 5;
        SystemSettings::update(&services.db, settings)
            .await
            .expect("update settings to target dimension");
        services.embedding_provider =
            Arc::new(EmbeddingProvider::new_hashed(5).expect("target provider"));

        prepare_embedding_runtime(&services, EmbeddingRuntimeRole::Maintainer)
            .await
            .expect("maintainer startup with data");

        assert_eq!(
            embedding_index_dimension(&services.db)
                .await
                .expect("index dim"),
            Some(5)
        );

        let rows: Vec<serde_json::Value> = services
            .db
            .client
            .query("SELECT embedding FROM text_chunk_embedding;")
            .await
            .expect("query embeddings")
            .take(0)
            .expect("take rows");
        let row = rows
            .first()
            .expect("exactly one embedding row after re-embed");
        let embedding = row
            .get("embedding")
            .and_then(|v| v.as_array())
            .expect("embedding array");
        assert_eq!(
            embedding.len(),
            5,
            "stored vectors should match the new provider dimension"
        );

        tokio::fs::remove_dir_all(&data_dir).await.ok();
    }

    #[tokio::test]
    async fn reembed_lock_is_exclusive_and_reusable_after_release() {
        let db = test_db().await;

        let first = reembed_lock_owner();
        assert!(
            try_acquire_reembed_lock(&db, &first)
                .await
                .expect("acquire first"),
            "the first acquirer should win the lock"
        );

        // A second, concurrent maintainer must not be able to take a held lock.
        let second = format!("{first}-peer");
        assert!(
            !try_acquire_reembed_lock(&db, &second)
                .await
                .expect("contend for lock"),
            "a held lock must not be granted to another owner"
        );

        // Releasing it (only the holder can) frees it for the next maintainer.
        release_reembed_lock(&db, &first).await;
        assert!(
            try_acquire_reembed_lock(&db, &second)
                .await
                .expect("re-acquire after release"),
            "the lock should be grantable again once released"
        );
    }
}
