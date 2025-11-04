use std::time::Instant;

use anyhow::{anyhow, Context};
use common::storage::types::system_settings::SystemSettings;
use tracing::{info, warn};

use crate::{
    db_helpers::{recreate_indexes, remove_all_indexes, reset_namespace},
    eval::{
        can_reuse_namespace, cases_from_manifest, enforce_system_settings, ensure_eval_user,
        record_namespace_state, warm_hnsw_cache,
    },
    ingest,
};

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{CorpusReady, EvaluationMachine, NamespaceReady},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn prepare_namespace(
    machine: EvaluationMachine<(), CorpusReady>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<NamespaceReady> {
    let stage = EvalStage::PrepareNamespace;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    let config = ctx.config();
    let dataset = ctx.dataset();
    let expected_fingerprint = ctx
        .expected_fingerprint
        .as_deref()
        .unwrap_or_default()
        .to_string();
    let namespace = ctx.namespace.clone();
    let database = ctx.database.clone();
    let embedding_provider = ctx.embedding_provider().clone();

    let mut namespace_reused = false;
    if !config.reseed_slice {
        namespace_reused = {
            let slice = ctx.slice();
            can_reuse_namespace(
                ctx.db(),
                ctx.descriptor(),
                &namespace,
                &database,
                dataset.metadata.id.as_str(),
                slice.manifest.slice_id.as_str(),
                expected_fingerprint.as_str(),
                slice.manifest.case_count,
            )
            .await?
        };
    }

    let mut namespace_seed_ms = None;
    if !namespace_reused {
        ctx.must_reapply_settings = true;
        if let Err(err) = reset_namespace(ctx.db(), &namespace, &database).await {
            warn!(
                error = %err,
                namespace,
                database = %database,
                "Failed to reset namespace before reseeding; continuing with existing data"
            );
        } else if let Err(err) = ctx.db().apply_migrations().await {
            warn!(error = %err, "Failed to reapply migrations after namespace reset");
        }

        {
            let slice = ctx.slice();
            info!(
                slice = slice.manifest.slice_id.as_str(),
                window_offset = ctx.window_offset,
                window_length = ctx.window_length,
                positives = slice.manifest.positive_paragraphs,
                negatives = slice.manifest.negative_paragraphs,
                total = slice.manifest.total_paragraphs,
                "Seeding ingestion corpus into SurrealDB"
            );
        }
        let indexes_disabled = remove_all_indexes(ctx.db()).await.is_ok();
        let seed_start = Instant::now();
        ingest::seed_manifest_into_db(ctx.db(), &ctx.corpus_handle().manifest)
            .await
            .context("seeding ingestion corpus from manifest")?;
        namespace_seed_ms = Some(seed_start.elapsed().as_millis() as u128);
        if indexes_disabled {
            info!("Recreating indexes after namespace reset");
            if let Err(err) = recreate_indexes(ctx.db(), embedding_provider.dimension()).await {
                warn!(error = %err, "failed to restore indexes after namespace reset");
            } else {
                warm_hnsw_cache(ctx.db(), embedding_provider.dimension()).await?;
            }
        }
        {
            let slice = ctx.slice();
            record_namespace_state(
                ctx.descriptor(),
                dataset.metadata.id.as_str(),
                slice.manifest.slice_id.as_str(),
                expected_fingerprint.as_str(),
                &namespace,
                &database,
                slice.manifest.case_count,
            )
            .await;
        }
    }

    if ctx.must_reapply_settings {
        let mut settings = SystemSettings::get_current(ctx.db())
            .await
            .context("reloading system settings after namespace reset")?;
        settings =
            enforce_system_settings(ctx.db(), settings, embedding_provider.dimension(), config)
                .await?;
        ctx.settings = Some(settings);
        ctx.must_reapply_settings = false;
    }

    let user = ensure_eval_user(ctx.db()).await?;
    ctx.eval_user = Some(user);

    let cases = cases_from_manifest(&ctx.corpus_handle().manifest);
    if cases.is_empty() {
        return Err(anyhow!(
            "no answerable questions found in converted dataset for evaluation"
        ));
    }
    ctx.cases = cases;
    ctx.namespace_reused = namespace_reused;
    ctx.namespace_seed_ms = namespace_seed_ms;

    info!(
        cases = ctx.cases.len(),
        window_offset = ctx.window_offset,
        namespace_reused = namespace_reused,
        "Dataset ready"
    );

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .prepare_namespace()
        .map_err(|(_, guard)| map_guard_error("prepare_namespace", guard))
}
