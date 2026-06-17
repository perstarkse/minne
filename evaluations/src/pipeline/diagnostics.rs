use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use crate::{args, types::CaseDiagnostics};

pub(crate) async fn write_chunk_diagnostics(path: &Path, cases: &[CaseDiagnostics]) -> Result<()> {
    args::ensure_parent(path)?;
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("creating diagnostics file {}", path.display()))?;
    for case in cases {
        let line = serde_json::to_vec(case).context("serialising chunk diagnostics entry")?;
        file.write_all(&line).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}
