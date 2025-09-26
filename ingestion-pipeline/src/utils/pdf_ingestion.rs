use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_openai::types::{
    ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, ImageDetail, ImageUrlArgs,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use headless_chrome::{
    protocol::cdp::{Emulation, Page, DOM},
    Browser,
};
use lopdf::Document;
use tokio::time::sleep;
use tracing::{debug, warn};

use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
    utils::config::PdfIngestMode,
};

const FAST_PATH_MIN_LEN: usize = 150;
const FAST_PATH_MIN_ASCII_RATIO: f64 = 0.7;
const MAX_VISION_PAGES: usize = 50;
const PAGES_PER_VISION_CHUNK: usize = 4;
const MAX_VISION_ATTEMPTS: usize = 2;
const PDF_MARKDOWN_PROMPT: &str = "Convert these PDF pages to clean Markdown. Preserve headings, lists, tables, blockquotes, code fences, and inline formatting. Keep the original reading order and avoid adding commentary.";
const PDF_MARKDOWN_PROMPT_RETRY: &str = "You must transcribe the provided PDF page images into accurate Markdown. The images are already supplied, so do not respond that you cannot view them. Extract all visible text, tables, and structure.";
const PDF_VISION_SYSTEM_PROMPT: &str = "You are a PDF transcription assistant. You can always see the provided page images. Always produce faithful Markdown and never claim you cannot view the document.";
const NAVIGATION_RETRY_INTERVAL_MS: u64 = 120;
const NAVIGATION_RETRY_ATTEMPTS: usize = 10;
const MIN_PAGE_IMAGE_BYTES: usize = 1_024;
const DEBUG_IMAGE_ENV_VAR: &str = "MINNE_PDF_DEBUG_DIR";

/// Attempts to extract PDF content, using a fast text layer first and falling back to
/// rendering the document for a vision-enabled LLM when needed.
pub async fn extract_pdf_content(
    file_path: &Path,
    db: &SurrealDbClient,
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    mode: &PdfIngestMode,
) -> Result<String, AppError> {
    let pdf_bytes = tokio::fs::read(file_path).await?;

    if let Some(candidate) = try_fast_path(pdf_bytes.clone()).await? {
        return Ok(candidate);
    }

    if matches!(mode, PdfIngestMode::Classic) {
        return Err(AppError::Processing(
            "PDF text extraction failed and LLM-first mode is disabled".into(),
        ));
    }

    let page_numbers = load_page_numbers(pdf_bytes.clone()).await?;
    if page_numbers.is_empty() {
        return Err(AppError::Processing("PDF appears to have no pages".into()));
    }

    if page_numbers.len() > MAX_VISION_PAGES {
        return Err(AppError::Processing(format!(
            "PDF has {} pages which exceeds the configured vision processing limit of {}",
            page_numbers.len(),
            MAX_VISION_PAGES
        )));
    }

    let rendered_pages = render_pdf_pages(file_path, &page_numbers).await?;
    let combined_markdown = vision_markdown(rendered_pages, db, client).await?;

    Ok(post_process(&combined_markdown))
}

/// Runs `pdf-extract` on the PDF bytes and validates the result with simple heuristics.
/// Returns `Ok(None)` when the text layer is missing or too noisy.
async fn try_fast_path(pdf_bytes: Vec<u8>) -> Result<Option<String>, AppError> {
    let extraction = tokio::task::spawn_blocking(move || {
        pdf_extract::extract_text_from_mem(&pdf_bytes).map(|s| s.trim().to_string())
    })
    .await?
    .map_err(|err| AppError::Processing(format!("Failed to extract text from PDF: {err}")))?;

    if extraction.is_empty() {
        return Ok(None);
    }

    if !looks_good_enough(&extraction) {
        return Ok(None);
    }

    Ok(Some(normalize_fast_text(&extraction)))
}

/// Parses the PDF structure to discover the available page numbers while keeping work off
/// the async executor.
async fn load_page_numbers(pdf_bytes: Vec<u8>) -> Result<Vec<u32>, AppError> {
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
async fn render_pdf_pages(file_path: &Path, pages: &[u32]) -> Result<Vec<Vec<u8>>, AppError> {
    let file_url = url::Url::from_file_path(file_path)
        .map_err(|_| AppError::Processing("Unable to construct PDF file URL".into()))?;

    let browser = create_browser()?;
    let tab = browser
        .new_tab()
        .map_err(|err| AppError::Processing(format!("Failed to create Chrome tab: {err}")))?;

    tab.set_default_timeout(Duration::from_secs(10));
    configure_tab(&tab)?;

    let mut captures = Vec::with_capacity(pages.len());

    for (idx, page) in pages.iter().enumerate() {
        let target = format!(
            "{}#page={}&toolbar=0&statusbar=0&zoom=page-fit",
            file_url, page
        );
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
            if attempt + 1 < NAVIGATION_RETRY_ATTEMPTS {
                sleep(Duration::from_millis(NAVIGATION_RETRY_INTERVAL_MS)).await;
            }
        }

        if !loaded {
            return Err(AppError::Processing(
                "Timed out waiting for Chrome to render PDF page".into(),
            ));
        }

        let viewer_element = wait_for_pdf_ready(&tab)?;
        tokio::time::sleep(Duration::from_millis(350)).await;

        let png = match viewer_element.capture_screenshot(Page::CaptureScreenshotFormatOption::Png)
        {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!(error = %err, page = *page, "Element screenshot failed; falling back to full page capture");
                let screenshot = tab
                    .call_method(Page::CaptureScreenshot {
                        format: Some(Page::CaptureScreenshotFormatOption::Png),
                        quality: None,
                        clip: None,
                        from_surface: Some(true),
                        capture_beyond_viewport: Some(true),
                        optimize_for_speed: Some(false),
                    })
                    .map_err(|inner_err| {
                        AppError::Processing(format!(
                            "Failed to capture PDF page (fallback): {inner_err}"
                        ))
                    })?;
                STANDARD.decode(screenshot.data).map_err(|decode_err| {
                    AppError::Processing(format!(
                        "Failed to decode PDF screenshot (fallback): {decode_err}"
                    ))
                })?
            }
        };

        debug!(
            page = *page,
            bytes = png.len(),
            plan_index = idx,
            "Captured PDF page screenshot"
        );

        if is_suspicious_image(png.len()) {
            warn!(
                page = *page,
                bytes = png.len(),
                "Screenshot size below threshold; check rendering output"
            );
        }

        if let Err(err) = maybe_dump_debug_image(*page, &png).await {
            warn!(
                page = *page,
                error = %err,
                "Failed to write debug screenshot to disk"
            );
        }

        captures.push(png);
    }

    Ok(captures)
}

/// Launches a headless Chrome instance that respects the existing feature flags.
fn create_browser() -> Result<Browser, AppError> {
    #[cfg(feature = "docker")]
    {
        let options = headless_chrome::LaunchOptionsBuilder::default()
            .sandbox(false)
            .build()
            .map_err(|err| AppError::Processing(format!("Failed to launch Chrome: {err}")))?;
        Browser::new(options)
            .map_err(|err| AppError::Processing(format!("Failed to start Chrome: {err}")))
    }
    #[cfg(not(feature = "docker"))]
    {
        Browser::default()
            .map_err(|err| AppError::Processing(format!("Failed to start Chrome: {err}")))
    }
}

/// Sends one or more rendered pages to the configured multimodal model and stitches the resulting Markdown chunks together.
async fn vision_markdown(
    rendered_pages: Vec<Vec<u8>>,
    db: &SurrealDbClient,
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<String, AppError> {
    let settings = SystemSettings::get_current(db).await?;
    let prompt = PDF_MARKDOWN_PROMPT;

    debug!(
        pages = rendered_pages.len(),
        "Preparing vision batches for PDF conversion"
    );

    let mut markdown_sections = Vec::with_capacity(rendered_pages.len());

    for (batch_idx, chunk) in rendered_pages.chunks(PAGES_PER_VISION_CHUNK).enumerate() {
        let total_image_bytes: usize = chunk.iter().map(|bytes| bytes.len()).sum();
        debug!(
            batch = batch_idx,
            pages = chunk.len(),
            bytes = total_image_bytes,
            "Encoding PDF images for vision batch"
        );

        let encoded_images: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(idx, png_bytes)| {
                let encoded = STANDARD.encode(png_bytes);
                if encoded.len() < 80 {
                    warn!(
                        batch = batch_idx,
                        page_index = idx,
                        encoded_bytes = encoded.len(),
                        "Encoded PDF image payload unusually small"
                    );
                }
                encoded
            })
            .collect();

        let mut batch_markdown: Option<String> = None;

        for attempt in 0..MAX_VISION_ATTEMPTS {
            let prompt_text = prompt_for_attempt(attempt, prompt);

            let mut content_parts = Vec::with_capacity(encoded_images.len() + 1);
            content_parts.push(
                ChatCompletionRequestMessageContentPartTextArgs::default()
                    .text(format!("{}\n\n{}", PDF_VISION_SYSTEM_PROMPT, prompt_text))
                    .build()?
                    .into(),
            );

            for encoded in &encoded_images {
                let image_url = format!("data:image/png;base64,{}", encoded);
                content_parts.push(
                    ChatCompletionRequestMessageContentPartImageArgs::default()
                        .image_url(
                            ImageUrlArgs::default()
                                .url(image_url)
                                .detail(ImageDetail::High)
                                .build()?,
                        )
                        .build()?
                        .into(),
                );
            }

            let request = CreateChatCompletionRequestArgs::default()
                .model(settings.image_processing_model.clone())
                .temperature(0.0)
                .messages([ChatCompletionRequestUserMessageArgs::default()
                    .content(content_parts)
                    .build()?
                    .into()])
                .max_tokens(6400_u32)
                .build()?;

            let response = client.chat().create(request).await?;
            let Some(choice) = response.choices.first() else {
                warn!(
                    batch = batch_idx,
                    attempt, "Vision response contained zero choices"
                );
                continue;
            };

            let Some(content) = choice.message.content.as_ref() else {
                warn!(
                    batch = batch_idx,
                    attempt, "Vision response missing content field"
                );
                continue;
            };

            debug!(
                batch = batch_idx,
                attempt,
                response_chars = content.len(),
                "Received Markdown response for PDF batch"
            );

            let preview: String = if content.len() > 500 {
                let mut snippet = content.chars().take(500).collect::<String>();
                snippet.push_str("…");
                snippet
            } else {
                content.clone()
            };
            debug!(batch = batch_idx, attempt, preview = %preview, "Vision response content preview");

            if is_low_quality_response(content) {
                warn!(
                    batch = batch_idx,
                    attempt, "Vision model returned low quality response"
                );
                if attempt + 1 == MAX_VISION_ATTEMPTS {
                    return Err(AppError::Processing(
                        "Vision model failed to transcribe PDF page contents".into(),
                    ));
                }
                continue;
            }

            batch_markdown = Some(content.trim().to_string());
            break;
        }

        if let Some(markdown) = batch_markdown {
            markdown_sections.push(markdown);
        } else {
            return Err(AppError::Processing(
                "Vision model did not return usable Markdown".into(),
            ));
        }
    }

    Ok(markdown_sections.join("\n\n"))
}

/// Heuristic that determines whether the fast-path text looks like well-formed prose.
fn looks_good_enough(text: &str) -> bool {
    if text.len() < FAST_PATH_MIN_LEN {
        return false;
    }

    let total_chars = text.chars().count() as f64;
    if total_chars == 0.0 {
        return false;
    }

    let ascii_chars = text.chars().filter(|c| c.is_ascii()).count() as f64;
    let ascii_ratio = ascii_chars / total_chars;
    if ascii_ratio < FAST_PATH_MIN_ASCII_RATIO {
        return false;
    }

    let letters = text.chars().filter(|c| c.is_alphabetic()).count() as f64;
    let letter_ratio = letters / total_chars;
    letter_ratio > 0.3
}

/// Normalizes fast-path output so downstream consumers see consistent Markdown.
fn normalize_fast_text(text: &str) -> String {
    reflow_markdown(text)
}

/// Cleans, trims, and reflows Markdown created by the LLM path.
fn post_process(markdown: &str) -> String {
    let cleaned = markdown.replace('\r', "");
    let trimmed = cleaned.trim();
    reflow_markdown(trimmed)
}

/// Joins hard-wrapped paragraph text while preserving structural Markdown lines.
fn reflow_markdown(input: &str) -> String {
    let mut paragraphs = Vec::new();
    let mut buffer: Vec<String> = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !buffer.is_empty() {
                paragraphs.push(buffer.join(" "));
                buffer.clear();
            }
            continue;
        }

        if is_structural_line(trimmed) {
            if !buffer.is_empty() {
                paragraphs.push(buffer.join(" "));
                buffer.clear();
            }
            paragraphs.push(trimmed.to_string());
            continue;
        }

        buffer.push(trimmed.to_string());
    }

    if !buffer.is_empty() {
        paragraphs.push(buffer.join(" "));
    }

    paragraphs.join("\n\n")
}

/// Detects whether a line is structural Markdown that should remain on its own.
fn is_structural_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    line.starts_with('#')
        || line.starts_with('-')
        || line.starts_with('*')
        || line.starts_with('>')
        || line.starts_with("```")
        || line.starts_with('~')
        || line.starts_with("| ")
        || line.starts_with("+-")
        || lowered
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            && lowered.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_good_enough_short_text() {
        assert!(!looks_good_enough("too short"));
    }

    #[test]
    fn test_looks_good_enough_ascii_text() {
        let text = "This is a reasonably long ASCII text that should pass the heuristic. \
        It contains multiple sentences and a decent amount of letters to satisfy the threshold.";
        assert!(looks_good_enough(text));
    }

    #[test]
    fn test_reflow_markdown_preserves_lists() {
        let input = "Item one\nItem two\n\n- Bullet\n- Another";
        let output = reflow_markdown(input);
        assert!(output.contains("Item one Item two"));
        assert!(output.contains("- Bullet"));
    }

    #[test]
    fn test_debug_dump_directory_env_var() {
        std::env::remove_var(DEBUG_IMAGE_ENV_VAR);
        assert!(debug_dump_directory().is_none());

        std::env::set_var(DEBUG_IMAGE_ENV_VAR, "/tmp/minne_pdf_debug");
        let dir = debug_dump_directory().expect("expected debug directory");
        assert_eq!(dir, PathBuf::from("/tmp/minne_pdf_debug"));

        std::env::remove_var(DEBUG_IMAGE_ENV_VAR);
    }

    #[test]
    fn test_is_suspicious_image_threshold() {
        assert!(is_suspicious_image(0));
        assert!(is_suspicious_image(MIN_PAGE_IMAGE_BYTES - 1));
        assert!(!is_suspicious_image(MIN_PAGE_IMAGE_BYTES + 1));
    }

    #[test]
    fn test_is_low_quality_response_detection() {
        assert!(is_low_quality_response(""));
        assert!(is_low_quality_response("I'm unable to help."));
        assert!(is_low_quality_response("I cannot read this."));
        assert!(!is_low_quality_response("# Heading\nValid content"));
    }

    #[test]
    fn test_prompt_for_attempt_variants() {
        assert_eq!(
            prompt_for_attempt(0, PDF_MARKDOWN_PROMPT),
            PDF_MARKDOWN_PROMPT
        );
        assert_eq!(
            prompt_for_attempt(1, PDF_MARKDOWN_PROMPT),
            PDF_MARKDOWN_PROMPT_RETRY
        );
        assert_eq!(
            prompt_for_attempt(5, PDF_MARKDOWN_PROMPT),
            PDF_MARKDOWN_PROMPT_RETRY
        );
    }
}

fn debug_dump_directory() -> Option<PathBuf> {
    std::env::var(DEBUG_IMAGE_ENV_VAR)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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

fn wait_for_pdf_ready<'a>(
    tab: &'a headless_chrome::Tab,
) -> Result<headless_chrome::Element<'a>, AppError> {
    let element = tab
        .wait_for_element_with_custom_timeout("canvas", Duration::from_secs(6))
        .or_else(|_| {
            tab.wait_for_element_with_custom_timeout(
                "embed[type='application/pdf']",
                Duration::from_secs(6),
            )
        })
        .or_else(|_| tab.wait_for_element_with_custom_timeout("embed", Duration::from_secs(6)))
        .map_err(|err| AppError::Processing(format!("Timed out waiting for PDF content: {err}")))?;

    if let Err(err) = element.scroll_into_view() {
        debug!("Failed to scroll PDF element into view: {err}");
    }

    Ok(element)
}

fn is_suspicious_image(len: usize) -> bool {
    len < MIN_PAGE_IMAGE_BYTES
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

fn is_low_quality_response(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lowered = trimmed.to_ascii_lowercase();
    lowered.contains("unable to") || lowered.contains("cannot")
}

fn prompt_for_attempt(attempt: usize, base_prompt: &str) -> &str {
    if attempt == 0 {
        base_prompt
    } else {
        PDF_MARKDOWN_PROMPT_RETRY
    }
}
