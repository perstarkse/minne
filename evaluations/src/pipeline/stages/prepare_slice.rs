use std::time::Instant;

use anyhow::Context;
use tracing::info;

use crate::{
    eval::{default_database, default_namespace, ledger_target},
    slice,
};

use super::super::{
    context::{EvalStage, EvaluationContext},
    state::{EvaluationMachine, Ready, SlicePrepared},
};
use super::{map_guard_error, StageResult};

pub(crate) async fn prepare_slice(
    machine: EvaluationMachine<(), Ready>,
    ctx: &mut EvaluationContext<'_>,
) -> StageResult<SlicePrepared> {
    let stage = EvalStage::PrepareSlice;
    info!(
        evaluation_stage = stage.label(),
        "starting evaluation stage"
    );
    let started = Instant::now();

    let ledger_limit = ledger_target(ctx.config());
    let slice_settings = slice::slice_config_with_limit(ctx.config(), ledger_limit);
    let resolved_slice =
        slice::resolve_slice(ctx.dataset(), &slice_settings).context("resolving dataset slice")?;
    let window = slice::select_window(
        &resolved_slice,
        ctx.config().slice_offset,
        ctx.config().limit,
    )
    .context("selecting slice window (use --slice-grow to extend the ledger first)")?;

    ctx.ledger_limit = ledger_limit;
    ctx.slice_settings = Some(slice_settings);
    ctx.slice = Some(resolved_slice.clone());
    ctx.window_offset = window.offset;
    ctx.window_length = window.length;
    ctx.window_total_cases = window.total_cases;

    ctx.namespace = ctx
        .config()
        .database
        .db_namespace
        .clone()
        .unwrap_or_else(|| default_namespace(ctx.dataset().metadata.id.as_str(), ctx.config().limit));
    ctx.database = ctx
        .config()
        .database
        .db_database
        .clone()
        .unwrap_or_else(default_database);

    let elapsed = started.elapsed();
    ctx.record_stage_duration(stage, elapsed);
    info!(
        evaluation_stage = stage.label(),
        duration_ms = elapsed.as_millis(),
        "completed evaluation stage"
    );

    machine
        .prepare_slice()
        .map_err(|(_, guard)| map_guard_error("prepare_slice", guard))
}
