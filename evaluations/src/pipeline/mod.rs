mod context;
mod stages;
mod state;

use anyhow::Result;

use crate::{args::Config, datasets::ConvertedDataset, types::EvaluationSummary};

use context::EvaluationContext;

pub async fn run_evaluation(
    dataset: &ConvertedDataset,
    config: &Config,
) -> Result<EvaluationSummary> {
    let mut ctx = EvaluationContext::new(dataset, config);
    let machine = state::ready();

    let machine = stages::prepare_slice(machine, &mut ctx).await?;
    let machine = stages::prepare_db(machine, &mut ctx).await?;
    let machine = stages::prepare_corpus(machine, &mut ctx).await?;
    let machine = stages::prepare_namespace(machine, &mut ctx).await?;
    let machine = stages::run_queries(machine, &mut ctx).await?;
    let machine = stages::summarize(machine, &mut ctx).await?;
    let machine = stages::finalize(machine, &mut ctx).await?;

    drop(machine);

    Ok(ctx.into_summary())
}
