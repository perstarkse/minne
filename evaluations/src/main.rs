mod args;
mod cases;
mod cli;
mod context_stats;
mod corpus;
mod datasets;
mod db;
mod inspection;
mod openai;
mod perf;
mod pipeline;
mod report;
mod settings;
mod slice;
mod types;

use anyhow::Context;
use tokio::runtime::Builder;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

/// Configure `SurrealDB` environment variables for optimal performance
#[allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]
fn configure_surrealdb_performance(cpu_count: usize) {
    let indexing_batch_size = std::env::var("SURREAL_INDEXING_BATCH_SIZE")
        .unwrap_or_else(|_| (cpu_count * 2).to_string());
    let max_order_queue = std::env::var("SURREAL_MAX_ORDER_LIMIT_PRIORITY_QUEUE_SIZE")
        .unwrap_or_else(|_| (cpu_count * 4).to_string());
    let websocket_concurrent = std::env::var("SURREAL_WEBSOCKET_MAX_CONCURRENT_REQUESTS")
        .unwrap_or_else(|_| cpu_count.to_string());
    let websocket_buffer = std::env::var("SURREAL_WEBSOCKET_RESPONSE_BUFFER_SIZE")
        .unwrap_or_else(|_| (cpu_count * 8).to_string());
    let transaction_cache = std::env::var("SURREAL_TRANSACTION_CACHE_SIZE")
        .unwrap_or_else(|_| (cpu_count * 16).to_string());
    // SAFETY: single-threaded setup before SurrealDB clients are created.
    unsafe {
        std::env::set_var("SURREAL_INDEXING_BATCH_SIZE", indexing_batch_size);
        std::env::set_var(
            "SURREAL_MAX_ORDER_LIMIT_PRIORITY_QUEUE_SIZE",
            max_order_queue,
        );
        std::env::set_var(
            "SURREAL_WEBSOCKET_MAX_CONCURRENT_REQUESTS",
            websocket_concurrent,
        );
        std::env::set_var("SURREAL_WEBSOCKET_RESPONSE_BUFFER_SIZE", websocket_buffer);
        std::env::set_var("SURREAL_TRANSACTION_CACHE_SIZE", transaction_cache);
    }

    info!(
        indexing_batch_size = %std::env::var("SURREAL_INDEXING_BATCH_SIZE").unwrap(),
        max_order_queue = %std::env::var("SURREAL_MAX_ORDER_LIMIT_PRIORITY_QUEUE_SIZE").unwrap(),
        websocket_concurrent = %std::env::var("SURREAL_WEBSOCKET_MAX_CONCURRENT_REQUESTS").unwrap(),
        websocket_buffer = %std::env::var("SURREAL_WEBSOCKET_RESPONSE_BUFFER_SIZE").unwrap(),
        transaction_cache = %std::env::var("SURREAL_TRANSACTION_CACHE_SIZE").unwrap(),
        "Configured SurrealDB performance variables"
    );
}

fn main() -> anyhow::Result<()> {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(std::thread::available_parallelism()?.get())
        .max_blocking_threads(std::thread::available_parallelism()?.get())
        .thread_stack_size(10 * 1024 * 1024)
        .thread_name("eval-retrieval-worker")
        .build()
        .context("failed to create tokio runtime")?;

    runtime.block_on(async_main())
}

#[allow(clippy::too_many_lines)]
async fn async_main() -> anyhow::Result<()> {
    let cpu_count = std::thread::available_parallelism()?.get();
    info!(
        cpu_cores = cpu_count,
        worker_threads = cpu_count,
        blocking_threads = cpu_count,
        thread_stack_size = "10MiB",
        "Started multi-threaded tokio runtime"
    );

    configure_surrealdb_performance(cpu_count);

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = fmt()
        .with_env_filter(EnvFilter::try_new(&filter).unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();

    let parsed = args::parse()?;

    if parsed.config.inspect_question.is_some() {
        inspection::inspect_question(&parsed.config).await?;
        return Ok(());
    }

    if parsed.config.status {
        let status = cli::collect_status(&parsed.config).await?;
        cli::print_status(&status);
        return Ok(());
    }

    if parsed.config.warm {
        cli::warm(&parsed.config).await?;
        return Ok(());
    }

    let dataset_kind = parsed.config.dataset;

    if parsed.config.convert_only {
        info!(
            dataset = dataset_kind.id(),
            "Starting dataset conversion only run"
        );
        let dataset = crate::datasets::convert(
            parsed.config.raw_dataset_path.as_path(),
            dataset_kind,
            parsed.config.llm_mode,
        )
        .with_context(|| {
            format!(
                "converting {} dataset at {}",
                dataset_kind.label(),
                parsed.config.raw_dataset_path.display()
            )
        })?;
        let store_dir = datasets::store_dir_for(&parsed.config.converted_dataset_path);
        datasets::write_sharded(&dataset, &store_dir)?;
        datasets::prebuild_catalog_slices(&dataset, &parsed.config)?;
        println!("Converted dataset written under {}", store_dir.display());
        return Ok(());
    }

    if parsed.config.require_ready {
        cli::ensure_query_ready(&parsed.config).await?;
    }

    info!(dataset = dataset_kind.id(), "Preparing converted dataset");
    let loaded =
        crate::datasets::prepare_dataset(dataset_kind, &parsed.config).with_context(|| {
            format!(
                "preparing converted dataset at {}",
                parsed.config.converted_dataset_path.display()
            )
        })?;

    info!(
        questions = loaded
            .dataset
            .paragraphs
            .iter()
            .map(|p| p.questions.len())
            .sum::<usize>(),
        paragraphs = loaded.dataset.paragraphs.len(),
        partial = loaded.partial,
        dataset = loaded.dataset.metadata.id.as_str(),
        "Dataset ready"
    );

    if parsed.config.slice_grow.is_some() {
        slice::grow_slice(&loaded.dataset, &parsed.config).context("growing slice ledger")?;
        return Ok(());
    }

    info!("Running retrieval evaluation");
    let summary = pipeline::run_evaluation(
        &loaded.dataset,
        &parsed.config,
        Some(loaded.content_checksum.as_str()),
    )
    .await
    .context("running retrieval evaluation")?;

    let report = report::write_reports(
        &summary,
        parsed.config.report_dir.as_path(),
        parsed.config.summary_sample,
    )
    .with_context(|| format!("writing reports to {}", parsed.config.report_dir.display()))?;
    let perf_mirrors = perf::mirror_perf_outputs(
        &report.record,
        &summary,
        parsed.config.report_dir.as_path(),
        parsed.config.perf_log_json.as_deref(),
        parsed.config.perf_log_dir.as_deref(),
    )
    .with_context(|| {
        format!(
            "writing perf mirrors under {}",
            parsed.config.report_dir.display()
        )
    })?;

    let perf_note = if perf_mirrors.is_empty() {
        String::new()
    } else {
        format!(
            " | Perf mirrors: {}",
            perf_mirrors
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    if summary.llm_cases > 0 {
        println!(
            "[{}] Retrieval Precision@{k}: {precision:.3} ({correct}/{retrieval_total}) + LLM: {llm_answered}/{llm_total} ({llm_precision:.3}) → JSON: {json} | Markdown: {md} | History: {history}{perf_note}",
            summary.dataset_label,
            k = summary.k,
            precision = summary.precision,
            correct = summary.correct,
            retrieval_total = summary.retrieval_cases,
            llm_answered = summary.llm_answered,
            llm_total = summary.llm_cases,
            llm_precision = summary.llm_precision,
            json = report.paths.json.display(),
            md = report.paths.markdown.display(),
            history = report.history_path.display(),
            perf_note = perf_note,
        );
    } else {
        println!(
            "[{}] Retrieval Precision@{k}: {precision:.3} ({correct}/{retrieval_total}) | Retrieved context: {chunks} chunks, {tokens} tokens ({tokenizer}, avg {avg_tokens:.0}/query, p95 {p95}) → JSON: {json} | Markdown: {md} | History: {history}{perf_note}",
            summary.dataset_label,
            k = summary.k,
            precision = summary.precision,
            correct = summary.correct,
            retrieval_total = summary.retrieval_cases,
            chunks = summary.retrieved_context.total_chunks,
            tokens = summary.retrieved_context.total_tokens,
            tokenizer = summary.retrieved_context.tokenizer,
            avg_tokens = summary.retrieved_context.avg_tokens_per_query,
            p95 = summary.retrieved_context.p95_tokens_per_query,
            json = report.paths.json.display(),
            md = report.paths.markdown.display(),
            history = report.history_path.display(),
            perf_note = perf_note,
        );
    }

    if parsed.config.perf_log_console {
        perf::print_console_summary(&report.record);
    }

    Ok(())
}
