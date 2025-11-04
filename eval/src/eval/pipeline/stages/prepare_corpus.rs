use std::time::Instant;

use anyhow::Context;
use tracing::info;

use crate::{ingest, slice, snapshot};

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

    let eval_user_id = "eval-user".to_string();
    let ingestion_timer = Instant::now();
    let corpus_handle = {
        let slice = ctx.slice();
        let window = slice::select_window(slice, ctx.config().slice_offset, ctx.config().limit)
            .context("selecting slice window for corpus preparation")?;
        ingest::ensure_corpus(
            ctx.dataset(),
            slice,
            &window,
            &cache_settings,
            &embedding_provider,
            openai_client,
            &eval_user_id,
            config.converted_dataset_path.as_path(),
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
    ctx.descriptor = Some(snapshot::Descriptor::new(
        config,
        ctx.slice(),
        ctx.embedding_provider(),
    ));

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
