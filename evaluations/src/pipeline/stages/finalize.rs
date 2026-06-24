use std::time::Instant;

use anyhow::Context;
use tracing::info;

use super::super::{
    context::{EvalStage, EvaluationContext},
    diagnostics::write_chunk_diagnostics,
};

pub(crate) async fn finalize(ctx: &mut EvaluationContext<'_>) -> anyhow::Result<()> {
    let stage = EvalStage::Finalize;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    if let Some(path) = ctx.diagnostics_path.as_ref()
        && ctx.diagnostics_enabled
    {
        write_chunk_diagnostics(path.as_path(), &ctx.diagnostics_output)
            .await
            .with_context(|| format!("writing chunk diagnostics to {}", path.display()))?;
    }

    info!(
        total_cases = ctx.summary.as_ref().map_or(0, |s| s.total_cases),
        correct = ctx.summary.as_ref().map_or(0, |s| s.correct),
        precision = ctx.summary.as_ref().map_or(0.0, |s| s.precision),
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

    Ok(())
}
