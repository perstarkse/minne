use std::time::Instant;

use anyhow::Context;
use tracing::info;

use crate::eval::write_chunk_diagnostics;

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{Completed, EvaluationMachine, Summarized},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn finalize(
    machine: EvaluationMachine<(), Summarized>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<Completed> {
    let stage = EvalStage::Finalize;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    if let Some(cache) = ctx.embedding_cache.as_ref() {
        cache
            .persist()
            .await
            .context("persisting embedding cache")?;
    }

    if let Some(path) = ctx.diagnostics_path.as_ref() {
        if ctx.diagnostics_enabled {
            write_chunk_diagnostics(path.as_path(), &ctx.diagnostics_output)
                .await
                .with_context(|| format!("writing chunk diagnostics to {}", path.display()))?;
        }
    }

    info!(
        total_cases = ctx.summary.as_ref().map(|s| s.total_cases).unwrap_or(0),
        correct = ctx.summary.as_ref().map(|s| s.correct).unwrap_or(0),
        precision = ctx.summary.as_ref().map(|s| s.precision).unwrap_or(0.0),
        dataset = ctx.dataset().metadata.id.as_str(),
        "Evaluation complete"
    );

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .finalize()
        .map_err(|(_, guard)| map_guard_error("finalize", guard))
}
