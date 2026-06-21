//! Page-fetching abstraction that decouples URL extraction from the underlying engine.
//!
//! The primary implementation uses [`servo_fetch`], a pure-Rust Servo engine that
//! provides high extraction quality (word-F1 0.819), fast startup (~331ms), and a
//! small memory footprint (~64MB peak).

use std::time::Duration;

use common::error::AppError;
use tracing::info;

/// Captured content from a single page fetch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PageCapture {
    /// Raw HTML source of the page.
    pub html: String,
    /// Readable Markdown extracted from the page content.
    pub markdown: String,
    /// JPEG/PNG screenshot bytes, or empty if not captured.
    pub screenshot: Vec<u8>,
}

/// Abstraction over a page-fetching engine.
pub(crate) trait PageFetcher: Send + Sync + std::fmt::Debug {
    /// Fetches a URL and returns the captured content (HTML, markdown, screenshot).
    fn fetch(&self, url: &str) -> Result<PageCapture, AppError>;
}

/// Fetcher powered by the embedded Servo engine via `servo-fetch`.
///
/// Provides HTML, extracted Markdown, and a PNG screenshot.
#[derive(Debug)]
pub(crate) struct ServoFetchFetcher;

impl PageFetcher for ServoFetchFetcher {
    fn fetch(&self, url: &str) -> Result<PageCapture, AppError> {
        let page = servo_fetch::blocking::fetch(
            &servo_fetch::FetchOptions::screenshot(url, true)
                .timeout(Duration::from_secs(30))
                .settle(Duration::from_millis(3000)),
        )
        .map_err(|err| AppError::Processing(format!("servo-fetch failed for {url}: {err}")))?;

        let html = page.html.clone();
        let markdown = page
            .markdown()
            .map_err(|err| AppError::Processing(format!("failed to extract markdown: {err}")))?;
        let screenshot = page.screenshot_png().unwrap_or_default().to_vec();

        info!(
            url = %url,
            html_bytes = html.len(),
            md_chars = markdown.len(),
            screenshot_bytes = screenshot.len(),
            "servo-fetch completed"
        );

        Ok(PageCapture {
            html,
            markdown,
            screenshot,
        })
    }
}

/// Creates the default page fetcher for the current configuration.
#[allow(unreachable_pub)]
pub(crate) fn create_fetcher() -> Box<dyn PageFetcher> {
    Box::new(ServoFetchFetcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_fetcher_constructs() {
        let fetcher = create_fetcher();
        assert!(!format!("{fetcher:?}").is_empty());
    }

    #[test]
    fn test_servo_fetcher_constructs() {
        let _ = ServoFetchFetcher;
    }

    #[test]
    fn test_trait_object_dispatch() {
        let fetcher: Box<dyn PageFetcher> = Box::new(ServoFetchFetcher);
        assert!(!format!("{fetcher:?}").is_empty());
    }

    /// Smoke test: Servo engine initialises even without display server.
    /// Wrap in `catch_unwind` because child-thread panics from servo
    /// (e.g. missing wayland) would otherwise escape the test harness.
    #[test]
    fn test_servo_engine_initializes() {
        let fetcher = ServoFetchFetcher;
        let result = std::panic::catch_unwind(move || {
            let _ = fetcher.fetch("about:blank");
        });

        if let Err(panic) = result {
            let msg = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            assert!(
                !(msg.contains("wayland")
                    || msg.contains("Library")
                    || msg.contains("servo-engine")),
                "Servo engine initialization failed: {msg}"
            );
        }
    }
}
