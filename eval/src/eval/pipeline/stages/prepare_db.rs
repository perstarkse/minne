use std::{sync::Arc, time::Instant};

use anyhow::{anyhow, Context};
use tracing::info;

use crate::{
    args::EmbeddingBackend,
    cache::EmbeddingCache,
    embedding,
    eval::{
        connect_eval_db, enforce_system_settings, load_or_init_system_settings, sanitize_model_code,
    },
    openai,
};

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{DbReady, EvaluationMachine, SlicePrepared},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn prepare_db(
    machine: EvaluationMachine<(), SlicePrepared>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<DbReady> {
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
    let (mut settings, settings_missing) = load_or_init_system_settings(&db).await?;

    let embedding_provider =
        embedding::build_provider(config, settings.embedding_dimensions as usize)
            .await
            .context("building embedding provider")?;
    let (raw_openai_client, openai_base_url) =
        openai::build_client_from_env().context("building OpenAI client")?;
    let openai_client = Arc::new(raw_openai_client);
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
    info!(openai_base_url = %openai_base_url, "OpenAI client configured");

    let embedding_cache = if config.embedding_backend == EmbeddingBackend::FastEmbed {
        if let Some(model_code) = embedding_provider.model_code() {
            let sanitized = sanitize_model_code(&model_code);
            let path = config.cache_dir.join(format!("{sanitized}.json"));
            if config.force_convert && path.exists() {
                tokio::fs::remove_file(&path)
                    .await
                    .with_context(|| format!("removing stale cache {}", path.display()))
                    .ok();
            }
            let cache = EmbeddingCache::load(&path).await?;
            info!(path = %path.display(), "Embedding cache ready");
            Some(cache)
        } else {
            None
        }
    } else {
        None
    };

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
    ctx.embedding_cache = embedding_cache;
    ctx.openai_client = Some(openai_client);
    ctx.openai_base_url = Some(openai_base_url);

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .prepare_db()
        .map_err(|(_, guard)| map_guard_error("prepare_db", guard))
}
