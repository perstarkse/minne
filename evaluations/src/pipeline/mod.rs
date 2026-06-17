mod context;
mod diagnostics;
mod stages;

use anyhow::Result;

use crate::{args::Config, datasets::ConvertedDataset, types::EvaluationSummary};

use context::EvaluationContext;

async fn run_through_namespace<'a>(
    dataset: &'a ConvertedDataset,
    config: &'a Config,
    content_checksum: Option<String>,
) -> Result<EvaluationContext<'a>> {
    let mut ctx = EvaluationContext::new(dataset, config, content_checksum);
    stages::prepare_slice(&mut ctx).await?;
    stages::prepare_db(&mut ctx).await?;
    stages::prepare_corpus(&mut ctx).await?;
    stages::prepare_namespace(&mut ctx).await?;
    Ok(ctx)
}

pub async fn warm_evaluation(
    dataset: &ConvertedDataset,
    config: &Config,
    content_checksum: &str,
) -> Result<()> {
    let _ctx = run_through_namespace(
        dataset,
        config,
        Some(content_checksum.to_string()),
    )
    .await?;
    Ok(())
}

pub async fn run_evaluation(
    dataset: &ConvertedDataset,
    config: &Config,
    content_checksum: Option<&str>,
) -> Result<EvaluationSummary> {
    let mut ctx = EvaluationContext::new(
        dataset,
        config,
        content_checksum.map(str::to_string),
    );
    stages::prepare_slice(&mut ctx).await?;
    stages::prepare_db(&mut ctx).await?;
    stages::prepare_corpus(&mut ctx).await?;
    stages::prepare_namespace(&mut ctx).await?;
    stages::run_queries(&mut ctx).await?;
    stages::summarize(&mut ctx).await?;
    stages::finalize(&mut ctx).await?;
    ctx.into_summary()
}
