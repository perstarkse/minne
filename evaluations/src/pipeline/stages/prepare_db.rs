use std::time::Instant;

use anyhow::{Context, anyhow};
use tracing::info;

use crate::{
    args::EmbeddingBackend,
    db::{connect_eval_db, sanitize_model_code},
    openai,
    settings::{enforce_system_settings, load_or_init_system_settings},
};
use common::utils::embedding::{EmbeddingProvider, default_embedding_pool_size};

use super::super::context::{EvalStage, EvaluationContext};

pub(crate) async fn prepare_db(ctx: &mut EvaluationContext<'_>) -> anyhow::Result<()> {
    let stage = EvalStage::PrepareDb;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    let namespace = ctx.namespace.clone();
    let database = ctx.database.clone();
    let config = ctx.config();

    let db = connect_eval_db(config, &namespace, &database).await?;

    let (openai_client, openai_base_url) =
        openai::ingestion_openai_client(config.ingest.include_entities)
            .context("building OpenAI client for ingestion")?;

    let embedding_provider = match config.embedding_backend {
        EmbeddingBackend::FastEmbed => EmbeddingProvider::new_fastembed(
            config.embedding_model.clone(),
            default_embedding_pool_size(),
        )
        .await
        .context("creating FastEmbed provider")?,
        EmbeddingBackend::Hashed => {
            EmbeddingProvider::new_hashed(1536).context("creating Hashed provider")?
        }
    };
    let provider_dimension = embedding_provider.dimension();
    if provider_dimension == 0 {
        return Err(anyhow!(
            "embedding provider reported zero dimensions; cannot continue"
        ));
    }

    info!(
        backend = embedding_provider.backend_label(),
        model = embedding_provider
            .model_code()
            .as_deref()
            .unwrap_or("<none>"),
        dimension = provider_dimension,
        "Embedding provider initialised"
    );
    if let Some(base_url) = &openai_base_url {
        info!(openai_base_url = %base_url, "OpenAI client configured for entity ingestion");
    }

    let (mut settings, settings_missing) =
        load_or_init_system_settings(&db, provider_dimension).await?;

    if config.embedding_backend == EmbeddingBackend::FastEmbed
        && let Some(model_code) = embedding_provider.model_code()
    {
        let sanitized = sanitize_model_code(&model_code);
        let path = config.cache_dir.join(format!("{sanitized}.json"));
        if config.force_convert && path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("removing stale cache {}", path.display()))
                .ok();
        }
    }

    let must_reapply_settings = settings_missing;
    let defer_initial_enforce = settings_missing && !config.reseed_slice;
    if !defer_initial_enforce {
        settings = enforce_system_settings(&db, settings, provider_dimension, config).await?;
    }

    ctx.db = Some(db);
    ctx.settings_missing = settings_missing;
    ctx.must_reapply_settings = must_reapply_settings;
    ctx.settings = Some(settings);
    ctx.embedding_provider = Some(embedding_provider);
    ctx.openai_client = Some(openai_client);
    ctx.openai_base_url = openai_base_url;

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    Ok(())
}
