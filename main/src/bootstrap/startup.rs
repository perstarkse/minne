use anyhow::Context;
use common::{
    storage::{
        db::SurrealDbClient,
        indexes::ensure_runtime,
        types::{
            knowledge_entity::KnowledgeEntity, system_settings::SystemSettings,
            text_chunk::TextChunk,
        },
    },
    utils::embedding::EmbeddingProvider,
};
use tracing::{info, warn};

use super::SharedServices;

/// Syncs embedding settings, re-embeds stored vectors when dimensions change, and
/// ensures runtime indexes match the active embedding dimension.
pub async fn prepare_embedding_runtime(services: &SharedServices) -> anyhow::Result<SystemSettings> {
    let (settings, dimensions_changed) =
        SystemSettings::sync_from_embedding_provider(&services.db, &services.embedding_provider)
            .await
            .context("sync system settings from embedding provider")?;

    if dimensions_changed {
        re_embed_all(
            &services.db,
            &services.embedding_provider,
            settings.embedding_dimensions,
        )
        .await?;
    }

    ensure_runtime(
        &services.db,
        settings.embedding_dimensions as usize,
    )
    .await
    .context("ensure runtime indexes")?;

    Ok(settings)
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
    TextChunk::update_all_embeddings_with_provider(db, embedding_provider)
        .await
        .context("re-embed text chunks after embedding dimension change")?;

    info!("Re-embedding KnowledgeEntities");
    KnowledgeEntity::update_all_embeddings_with_provider(db, embedding_provider)
        .await
        .context("re-embed knowledge entities after embedding dimension change")?;

    info!("Re-embedding complete");
    Ok(())
}
