mod args;
mod cache;
mod datasets;
mod db_helpers;
mod embedding;
mod eval;
mod ingest;
mod inspection;
mod openai;
mod perf;
mod report;
mod slice;
mod slices;
mod snapshot;

use anyhow::Context;
use tokio::runtime::Builder;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

/// Configure SurrealDB environment variables for optimal performance
fn configure_surrealdb_performance(cpu_count: usize) {
    // Set environment variables only if they're not already set
    let indexing_batch_size = std::env::var("SURREAL_INDEXING_BATCH_SIZE")
        .unwrap_or_else(|_| (cpu_count * 2).to_string());
    std::env::set_var("SURREAL_INDEXING_BATCH_SIZE", indexing_batch_size);

    let max_order_queue = std::env::var("SURREAL_MAX_ORDER_LIMIT_PRIORITY_QUEUE_SIZE")
        .unwrap_or_else(|_| (cpu_count * 4).to_string());
    std::env::set_var(
        "SURREAL_MAX_ORDER_LIMIT_PRIORITY_QUEUE_SIZE",
        max_order_queue,
    );

    let websocket_concurrent = std::env::var("SURREAL_WEBSOCKET_MAX_CONCURRENT_REQUESTS")
        .unwrap_or_else(|_| cpu_count.to_string());
    std::env::set_var(
        "SURREAL_WEBSOCKET_MAX_CONCURRENT_REQUESTS",
        websocket_concurrent,
    );

    let websocket_buffer = std::env::var("SURREAL_WEBSOCKET_RESPONSE_BUFFER_SIZE")
        .unwrap_or_else(|_| (cpu_count * 8).to_string());
    std::env::set_var("SURREAL_WEBSOCKET_RESPONSE_BUFFER_SIZE", websocket_buffer);

    let transaction_cache = std::env::var("SURREAL_TRANSACTION_CACHE_SIZE")
        .unwrap_or_else(|_| (cpu_count * 16).to_string());
    std::env::set_var("SURREAL_TRANSACTION_CACHE_SIZE", transaction_cache);

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
    // Create an explicit multi-threaded runtime with optimized configuration
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(std::thread::available_parallelism()?.get())
        .max_blocking_threads(std::thread::available_parallelism()?.get())
        .thread_stack_size(10 * 1024 * 1024) // 10MiB stack size
        .thread_name("eval-retrieval-worker")
        .build()
        .context("failed to create tokio runtime")?;

    runtime.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    // Log runtime configuration
    let cpu_count = std::thread::available_parallelism()?.get();
    info!(
        cpu_cores = cpu_count,
        worker_threads = cpu_count,
        blocking_threads = cpu_count,
        thread_stack_size = "10MiB",
        "Started multi-threaded tokio runtime"
    );

    // Configure SurrealDB environment variables for better performance
    configure_surrealdb_performance(cpu_count);

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = fmt()
        .with_env_filter(EnvFilter::try_new(&filter).unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();

    let parsed = args::parse()?;

    if parsed.show_help {
        args::print_help();
        return Ok(());
    }

    if parsed.config.inspect_question.is_some() {
        inspection::inspect_question(&parsed.config).await?;
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
            parsed.config.context_token_limit(),
        )
        .with_context(|| {
            format!(
                "converting {} dataset at {}",
                dataset_kind.label(),
                parsed.config.raw_dataset_path.display()
            )
        })?;
        crate::datasets::write_converted(&dataset, parsed.config.converted_dataset_path.as_path())
            .with_context(|| {
                format!(
                    "writing converted dataset to {}",
                    parsed.config.converted_dataset_path.display()
                )
            })?;
        println!(
            "Converted dataset written to {}",
            parsed.config.converted_dataset_path.display()
        );
        return Ok(());
    }

    info!(dataset = dataset_kind.id(), "Preparing converted dataset");
    let dataset = crate::datasets::ensure_converted(
        dataset_kind,
        parsed.config.raw_dataset_path.as_path(),
        parsed.config.converted_dataset_path.as_path(),
        parsed.config.force_convert,
        parsed.config.llm_mode,
        parsed.config.context_token_limit(),
    )
    .with_context(|| {
        format!(
            "preparing converted dataset at {}",
            parsed.config.converted_dataset_path.display()
        )
    })?;

    info!(
        questions = dataset
            .paragraphs
            .iter()
            .map(|p| p.questions.len())
            .sum::<usize>(),
        paragraphs = dataset.paragraphs.len(),
        dataset = dataset.metadata.id.as_str(),
        "Dataset ready"
    );

    if parsed.config.slice_grow.is_some() {
        eval::grow_slice(&dataset, &parsed.config)
            .await
            .context("growing slice ledger")?;
        return Ok(());
    }

    info!("Running retrieval evaluation");
    let summary = eval::run_evaluation(&dataset, &parsed.config)
        .await
        .context("running retrieval evaluation")?;

    let report_paths = report::write_reports(
        &summary,
        parsed.config.report_dir.as_path(),
        parsed.config.summary_sample,
    )
    .with_context(|| format!("writing reports to {}", parsed.config.report_dir.display()))?;
    let perf_log_path = perf::write_perf_logs(
        &summary,
        parsed.config.report_dir.as_path(),
        parsed.config.perf_log_json.as_deref(),
        parsed.config.perf_log_dir.as_deref(),
    )
    .with_context(|| {
        format!(
            "writing perf logs under {}",
            parsed.config.report_dir.display()
        )
    })?;

    if summary.llm_cases > 0 {
        println!(
            "[{}] Retrieval Precision@{k}: {precision:.3} ({correct}/{retrieval_total}) + LLM: {llm_answered}/{llm_total} ({llm_precision:.3}) → JSON: {json} | Markdown: {md} | Perf: {perf}",
            summary.dataset_label,
            k = summary.k,
            precision = summary.precision,
            correct = summary.correct,
            retrieval_total = summary.retrieval_cases,
            llm_answered = summary.llm_answered,
            llm_total = summary.llm_cases,
            llm_precision = summary.llm_precision,
            json = report_paths.json.display(),
            md = report_paths.markdown.display(),
            perf = perf_log_path.display()
        );
    } else {
        println!(
            "[{}] Retrieval Precision@{k}: {precision:.3} ({correct}/{retrieval_total}) → JSON: {json} | Markdown: {md} | Perf: {perf}",
            summary.dataset_label,
            k = summary.k,
            precision = summary.precision,
            correct = summary.correct,
            retrieval_total = summary.retrieval_cases,
            json = report_paths.json.display(),
            md = report_paths.markdown.display(),
            perf = perf_log_path.display()
        );
    }

    if parsed.config.perf_log_console {
        perf::print_console_summary(&summary);
    }

    Ok(())
}
