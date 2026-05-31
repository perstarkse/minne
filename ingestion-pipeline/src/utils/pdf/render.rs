//! Headless-Chrome rasterization of PDF pages into PNG screenshots.
//!
//! This is the only Chrome-dependent part of PDF ingestion. It depends on the
//! browser's internal PDF-viewer shadow DOM, so it is inherently fragile across
//! Chrome upgrades; a full-page-capture fallback guards the common failure modes.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use headless_chrome::protocol::cdp::{Emulation, Page, DOM};
use lopdf::Document;
use serde_json::Value;
use tracing::{debug, warn};

use common::error::AppError;

use crate::utils::browser::launch_browser;

const NAVIGATION_RETRY_INTERVAL_MS: u64 = 120;
const NAVIGATION_RETRY_ATTEMPTS: usize = 10;
const MIN_PAGE_IMAGE_BYTES: usize = 1_024;
const DEFAULT_VIEWPORT_WIDTH: u32 = 1_248; // generous width to reduce horizontal clipping
const DEFAULT_VIEWPORT_HEIGHT: u32 = 1_800; // tall enough to capture full page at fit-to-width scale
const DEFAULT_DEVICE_SCALE_FACTOR: f64 = 1.0;
const CANVAS_VIEWPORT_ATTEMPTS: usize = 12;
const CANVAS_VIEWPORT_WAIT_MS: u64 = 200;
const DEBUG_IMAGE_ENV_VAR: &str = "MINNE_PDF_DEBUG_DIR";

/// Parses the PDF structure to discover the available page numbers while keeping work off
/// the async executor.
pub(super) async fn load_page_numbers(pdf_bytes: Vec<u8>) -> Result<Vec<u32>, AppError> {
    let pages = tokio::task::spawn_blocking(move || -> Result<Vec<u32>, AppError> {
        let document = Document::load_mem(&pdf_bytes)
            .map_err(|err| AppError::Processing(format!("Failed to parse PDF: {err}")))?;
        let mut page_numbers: Vec<u32> = document.get_pages().keys().copied().collect();
        page_numbers.sort_unstable();
        Ok(page_numbers)
    })
    .await??;

    Ok(pages)
}

/// Uses the existing headless Chrome dependency to rasterize the requested PDF pages into PNGs.
pub(super) async fn render_pdf_pages(
    file_path: &Path,
    pages: &[u32],
) -> Result<Vec<Vec<u8>>, AppError> {
    let file_path = file_path.to_path_buf();
    let pages = pages.to_vec();
    let page_numbers = pages.clone();
    let captures =
        tokio::task::spawn_blocking(move || render_pdf_pages_inner(&file_path, &pages)).await??;

    for (page_number, png) in page_numbers.iter().zip(captures.iter()) {
        if let Err(err) = maybe_dump_debug_image(*page_number, png).await {
            warn!(
                page = page_number,
                error = %err,
                "Failed to write debug screenshot to disk"
            );
        }
    }

    Ok(captures)
}

fn render_pdf_pages_inner(file_path: &Path, pages: &[u32]) -> Result<Vec<Vec<u8>>, AppError> {
    let file_url = url::Url::from_file_path(file_path)
        .map_err(|()| AppError::Processing("Unable to construct PDF file URL".into()))?;

    let browser = launch_browser()?;
    let tab = browser
        .new_tab()
        .map_err(|err| AppError::Processing(format!("Failed to create Chrome tab: {err}")))?;

    tab.set_default_timeout(Duration::from_secs(10));
    configure_tab(&tab)?;
    set_pdf_viewport(&tab)?;

    let mut captures = Vec::with_capacity(pages.len());

    for page in pages.iter().copied() {
        let target = format!("{file_url}#page={page}&toolbar=0&statusbar=0&zoom=page-fit");
        tab.navigate_to(&target)
            .map_err(|err| AppError::Processing(format!("Failed to navigate to PDF page: {err}")))?
            .wait_until_navigated()
            .map_err(|err| AppError::Processing(format!("Navigation to PDF page failed: {err}")))?;

        let mut loaded = false;
        for attempt in 0..NAVIGATION_RETRY_ATTEMPTS {
            if tab
                .wait_for_element("embed, canvas, body")
                .map(|_| ())
                .is_ok()
            {
                loaded = true;
                break;
            }
            if attempt < NAVIGATION_RETRY_ATTEMPTS.saturating_sub(1) {
                std::thread::sleep(Duration::from_millis(NAVIGATION_RETRY_INTERVAL_MS));
            }
        }

        if !loaded {
            return Err(AppError::Processing(
                "Timed out waiting for Chrome to render PDF page".into(),
            ));
        }

        wait_for_pdf_ready(&tab, page)?;
        std::thread::sleep(Duration::from_millis(350));

        prepare_pdf_viewer(&tab, page);

        let mut viewport: Option<Page::Viewport> = None;
        for attempt in 0..CANVAS_VIEWPORT_ATTEMPTS {
            match canvas_viewport_for_page(&tab, page) {
                Ok(Some(vp)) => {
                    viewport = Some(vp);
                    break;
                }
                Ok(None) => {
                    if attempt < CANVAS_VIEWPORT_ATTEMPTS.saturating_sub(1) {
                        std::thread::sleep(Duration::from_millis(CANVAS_VIEWPORT_WAIT_MS));
                    }
                }
                Err(err) => {
                    warn!(page, error = %err, "Failed to derive canvas viewport");
                    break;
                }
            }
        }

        let png = if let Some(clip) = viewport {
            match tab.call_method(Page::CaptureScreenshot {
                format: Some(Page::CaptureScreenshotFormatOption::Png),
                quality: None,
                clip: Some(clip),
                from_surface: Some(true),
                capture_beyond_viewport: Some(true),
                optimize_for_speed: Some(false),
            }) {
                Ok(data) => match STANDARD.decode(data.data) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        warn!(error = %err, page, "Failed to decode clipped screenshot; falling back to full page capture");
                        capture_full_page_png(&tab)?
                    }
                },
                Err(err) => {
                    warn!(error = %err, page, "Clipped screenshot failed; falling back to full page capture");
                    capture_full_page_png(&tab)?
                }
            }
        } else {
            warn!(
                page,
                "Unable to determine canvas viewport; capturing full page"
            );
            capture_full_page_png(&tab)?
        };

        debug!(page, bytes = png.len(), "Captured PDF page screenshot");

        if is_suspicious_image(png.len()) {
            warn!(
                page,
                bytes = png.len(),
                "Screenshot size below threshold; check rendering output"
            );
        }

        captures.push(png);
    }

    Ok(captures)
}

fn configure_tab(tab: &headless_chrome::Tab) -> Result<(), AppError> {
    tab.call_method(Emulation::SetDefaultBackgroundColorOverride {
        color: Some(DOM::RGBA {
            r: 255,
            g: 255,
            b: 255,
            a: Some(1.0),
        }),
    })
    .map_err(|err| {
        AppError::Processing(format!("Failed to configure Chrome page background: {err}"))
    })?;

    Ok(())
}

fn set_pdf_viewport(tab: &headless_chrome::Tab) -> Result<(), AppError> {
    tab.call_method(Emulation::SetDeviceMetricsOverride {
        width: DEFAULT_VIEWPORT_WIDTH,
        height: DEFAULT_VIEWPORT_HEIGHT,
        device_scale_factor: DEFAULT_DEVICE_SCALE_FACTOR,
        mobile: false,
        scale: None,
        screen_width: Some(DEFAULT_VIEWPORT_WIDTH),
        screen_height: Some(DEFAULT_VIEWPORT_HEIGHT),
        position_x: None,
        position_y: None,
        dont_set_visible_size: Some(false),
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })
    .map_err(|err| AppError::Processing(format!("Failed to configure Chrome viewport: {err}")))?;

    tab.call_method(Emulation::SetVisibleSize {
        width: DEFAULT_VIEWPORT_WIDTH,
        height: DEFAULT_VIEWPORT_HEIGHT,
    })
    .map_err(|err| AppError::Processing(format!("Failed to apply Chrome visible size: {err}")))?;

    Ok(())
}

fn wait_for_pdf_ready(
    tab: &headless_chrome::Tab,
    page_number: u32,
) -> Result<headless_chrome::Element<'_>, AppError> {
    let embed_selector = "embed[type='application/pdf']";
    let element = tab
        .wait_for_element_with_custom_timeout(embed_selector, Duration::from_secs(8))
        .or_else(|_| tab.wait_for_element_with_custom_timeout("embed", Duration::from_secs(8)))
        .map_err(|err| AppError::Processing(format!("Timed out waiting for PDF content: {err}")))?;

    if let Err(err) = element.scroll_into_view() {
        debug!("Failed to scroll PDF element into view: {err}");
    }

    debug!(page = page_number, "PDF viewer element located");

    Ok(element)
}

fn prepare_pdf_viewer(tab: &headless_chrome::Tab, page_number: u32) {
    let script = format!(
        r#"(function() {{
            const embed = document.querySelector('embed[type="application/pdf"]') || document.querySelector('embed');
            if (!embed || !embed.shadowRoot) return false;
            const viewer = embed.shadowRoot.querySelector('pdf-viewer');
            if (!viewer || !viewer.shadowRoot) return false;
            const app = viewer.shadowRoot.querySelector('viewer-app');
            if (app && app.shadowRoot) {{
                const toolbar = app.shadowRoot.querySelector('#toolbar');
                if (toolbar) {{ toolbar.style.display = 'none'; }}
            }}
            const page = viewer.shadowRoot.querySelector('viewer-page:nth-of-type({page_number})');
            if (page && page.scrollIntoView) {{
                page.scrollIntoView({{ block: 'start', inline: 'center' }});
            }}
            const canvas = viewer.shadowRoot.querySelector('canvas[aria-label="Page {page_number}"]');
            return !!canvas;
        }})()"#
    );

    match tab.evaluate(&script, false) {
        Ok(result) => {
            let ready = result
                .value
                .as_ref()
                .and_then(Value::as_bool)
                .unwrap_or(false);
            debug!(page = page_number, ready, "Prepared PDF viewer page");
        }
        Err(err) => {
            debug!(page = page_number, error = %err, "Unable to run PDF viewer preparation script");
        }
    }
}

fn canvas_viewport_for_page(
    tab: &headless_chrome::Tab,
    page_number: u32,
) -> Result<Option<Page::Viewport>, AppError> {
    let script = format!(
        r#"(function() {{
            const embed = document.querySelector('embed[type="application/pdf"]') || document.querySelector('embed');
            if (!embed || !embed.shadowRoot) return null;
            const viewer = embed.shadowRoot.querySelector('pdf-viewer');
            if (!viewer || !viewer.shadowRoot) return null;
            const canvas = viewer.shadowRoot.querySelector('canvas[aria-label="Page {page_number}"]');
            if (!canvas) return null;
            const rect = canvas.getBoundingClientRect();
            return {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }};
        }})()"#
    );

    let result = tab
        .evaluate(&script, false)
        .map_err(|err| AppError::Processing(format!("Failed to inspect PDF canvas: {err}")))?;

    let Some(value) = result.value else {
        return Ok(None);
    };

    if value.is_null() {
        return Ok(None);
    }

    let x = value
        .get("x")
        .and_then(Value::as_f64)
        .unwrap_or_default()
        .max(0.0);
    let y = value
        .get("y")
        .and_then(Value::as_f64)
        .unwrap_or_default()
        .max(0.0);
    let width = value
        .get("width")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    let height = value
        .get("height")
        .and_then(Value::as_f64)
        .unwrap_or_default();

    if width <= 0.0 || height <= 0.0 {
        return Ok(None);
    }

    debug!(
        page = page_number,
        x, y, width, height, "Derived canvas viewport"
    );

    Ok(Some(Page::Viewport {
        x,
        y,
        width,
        height,
        scale: 1.0,
    }))
}

fn capture_full_page_png(tab: &headless_chrome::Tab) -> Result<Vec<u8>, AppError> {
    let screenshot = tab
        .call_method(Page::CaptureScreenshot {
            format: Some(Page::CaptureScreenshotFormatOption::Png),
            quality: None,
            clip: None,
            from_surface: Some(true),
            capture_beyond_viewport: Some(true),
            optimize_for_speed: Some(false),
        })
        .map_err(|err| {
            AppError::Processing(format!("Failed to capture PDF page (fallback): {err}"))
        })?;

    STANDARD.decode(screenshot.data).map_err(|err| {
        AppError::Processing(format!("Failed to decode PDF screenshot (fallback): {err}"))
    })
}

const fn is_suspicious_image(len: usize) -> bool {
    len < MIN_PAGE_IMAGE_BYTES
}

fn debug_dump_directory() -> Option<PathBuf> {
    std::env::var(DEBUG_IMAGE_ENV_VAR)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

async fn maybe_dump_debug_image(page_index: u32, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(dir) = debug_dump_directory() {
        tokio::fs::create_dir_all(&dir).await?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let file_path = dir.join(format!("page-{page_index:04}-{timestamp}.png"));
        tokio::fs::write(&file_path, bytes).await?;
        debug!(?file_path, size = bytes.len(), "Wrote PDF debug screenshot");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{self};

    #[test]
    fn test_debug_dump_directory_env_var() -> anyhow::Result<()> {
        std::env::remove_var(DEBUG_IMAGE_ENV_VAR);
        assert!(debug_dump_directory().is_none());

        std::env::set_var(DEBUG_IMAGE_ENV_VAR, "/tmp/minne_pdf_debug");
        let dir =
            debug_dump_directory().ok_or_else(|| anyhow::anyhow!("expected debug directory"))?;
        assert_eq!(dir, PathBuf::from("/tmp/minne_pdf_debug"));

        std::env::remove_var(DEBUG_IMAGE_ENV_VAR);
        Ok(())
    }

    #[test]
    fn test_is_suspicious_image_threshold() {
        assert!(is_suspicious_image(0));
        assert!(is_suspicious_image(MIN_PAGE_IMAGE_BYTES - 1));
        assert!(!is_suspicious_image(MIN_PAGE_IMAGE_BYTES + 1));
    }
}
