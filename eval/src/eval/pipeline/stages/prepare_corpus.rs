use std::time::Instant;

use anyhow::Context;
use tracing::info;

use crate::{eval::can_reuse_namespace, ingest, slice, snapshot};

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{CorpusReady, DbReady, EvaluationMachine},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn prepare_corpus(
    machine: EvaluationMachine<(), DbReady>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<CorpusReady> {
    let stage = EvalStage::PrepareCorpus;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    let config = ctx.config();
    let cache_settings = ingest::CorpusCacheConfig::from(config);
    let embedding_provider = ctx.embedding_provider().clone();
    let openai_client = ctx.openai_client();
    let slice = ctx.slice();
    let window = slice::select_window(slice, ctx.config().slice_offset, ctx.config().limit)
        .context("selecting slice window for corpus preparation")?;

    let descriptor = snapshot::Descriptor::new(config, slice, ctx.embedding_provider());
    let ingestion_config = ingest::make_ingestion_config(config);
    let expected_fingerprint = ingest::compute_ingestion_fingerprint(
        ctx.dataset(),
        slice,
        config.converted_dataset_path.as_path(),
        &ingestion_config,
    )?;
    let base_dir = ingest::cached_corpus_dir(
        &cache_settings,
        ctx.dataset().metadata.id.as_str(),
        slice.manifest.slice_id.as_str(),
    );

    if !config.reseed_slice {
        let requested_cases = window.cases.len();
        if can_reuse_namespace(
            ctx.db(),
            &descriptor,
            &ctx.namespace,
            &ctx.database,
            ctx.dataset().metadata.id.as_str(),
            slice.manifest.slice_id.as_str(),
            expected_fingerprint.as_str(),
            requested_cases,
        )
        .await?
        {
            if let Some(manifest) = ingest::load_cached_manifest(&base_dir)? {
                info!(
                    cache = %base_dir.display(),
                    namespace = ctx.namespace.as_str(),
                    database = ctx.database.as_str(),
                    "Namespace already seeded; reusing cached corpus manifest"
                );
                let corpus_handle = ingest::corpus_handle_from_manifest(manifest, base_dir);
                ctx.corpus_handle = Some(corpus_handle);
                ctx.expected_fingerprint = Some(expected_fingerprint);
                ctx.ingestion_duration_ms = 0;
                ctx.descriptor = Some(descriptor);

                let elapsed = started.elapsed();
                ctx.record_stage_duration(stage, elapsed);
                info!(
                    evaluation_stage = stage.label(),
                    duration_ms = elapsed.as_millis(),
                    "completed evaluation stage"
                );

                return machine
                    .prepare_corpus()
                    .map_err(|(_, guard)| map_guard_error("prepare_corpus", guard));
            } else {
                info!(
                    cache = %base_dir.display(),
                    "Namespace reusable but cached manifest missing; regenerating corpus"
                );
            }
        }
    }

    let eval_user_id = "eval-user".to_string();
    let ingestion_timer = Instant::now();
    let corpus_handle = {
        ingest::ensure_corpus(
            ctx.dataset(),
            slice,
            &window,
            &cache_settings,
            embedding_provider.clone().into(),
            openai_client,
            &eval_user_id,
            config.converted_dataset_path.as_path(),
            ingestion_config.clone(),
        )
        .await
        .context("ensuring ingestion-backed corpus")?
    };
    let expected_fingerprint = corpus_handle
        .manifest
        .metadata
        .ingestion_fingerprint
        .clone();
    let ingestion_duration_ms = ingestion_timer.elapsed().as_millis() as u128;
    info!(
        cache = %corpus_handle.path.display(),
        reused_ingestion = corpus_handle.reused_ingestion,
        reused_embeddings = corpus_handle.reused_embeddings,
        positive_ingested = corpus_handle.positive_ingested,
        negative_ingested = corpus_handle.negative_ingested,
        "Ingestion corpus ready"
    );

    ctx.corpus_handle = Some(corpus_handle);
    ctx.expected_fingerprint = Some(expected_fingerprint);
    ctx.ingestion_duration_ms = ingestion_duration_ms;
    ctx.descriptor = Some(descriptor);

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .prepare_corpus()
        .map_err(|(_, guard)| map_guard_error("prepare_corpus", guard))
}
