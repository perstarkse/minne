//! PDF page rasterization using pdfium-render via pdfium-auto.
//!
//! Uses direct `PDFium` bindings for reliable, pixel-perfect page rendering —
//! starts in ~5ms, requires no display server, and produces consistent output
//! independent of PDF reader version. Each page is rendered at a generous
//! resolution and encoded as PNG for downstream LLM vision ingestion.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::ImageFormat;
use lopdf::Document;
use pdfium_render::prelude::PdfRenderConfig;
use tracing::{debug, warn};

use common::error::AppError;

const MIN_PAGE_IMAGE_BYTES: usize = 1_024;
const RENDER_TARGET_WIDTH: i32 = 1200;
const RENDER_MAX_HEIGHT: i32 = 2000;
const DEBUG_IMAGE_ENV_VAR: &str = "MINNE_PDF_DEBUG_DIR";

/// Parses the PDF structure to discover the available page numbers while keeping work off
/// the async executor.
pub(super) async fn load_page_numbers(pdf_bytes: Vec<u8>) -> Result<Vec<u32>, AppError> {
    let pages = tokio::task::spawn_blocking(move || -> Result<Vec<u32>, AppError> {
        let document = Document::load_mem(&pdf_bytes)
            .map_err(|err| AppError::Processing(format!("failed to parse PDF: {err}")))?;
        let mut page_numbers: Vec<u32> = document.get_pages().keys().copied().collect();
        page_numbers.sort_unstable();
        Ok(page_numbers)
    })
    .await??;

    Ok(pages)
}

/// Renders the requested PDF pages as PNG-encoded byte vectors using `PDFium`.
///
/// Work is offloaded to a blocking thread since `PDFium`'s C API is not async-safe.
pub(super) async fn render_pdf_pages(
    file_path: &Path,
    pages: &[u32],
) -> Result<Vec<Vec<u8>>, AppError> {
    let file_path = file_path.to_path_buf();
    let pages = pages.to_vec();
    let page_numbers = pages.clone();

    let captures = tokio::task::spawn_blocking(move || render_inner(&file_path, &pages)).await??;

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

/// Initializes `PDFium`, opens the file, and renders each requested page.
fn render_inner(file_path: &Path, pages: &[u32]) -> Result<Vec<Vec<u8>>, AppError> {
    let pdfium = pdfium_auto::bind_pdfium_silent()
        .map_err(|err| AppError::Processing(format!("failed to bind PDFium library: {err}")))?;

    let doc = pdfium
        .load_pdf_from_file(file_path, None)
        .map_err(|err| AppError::Processing(format!("failed to load PDF file: {err}")))?;

    let render_config = PdfRenderConfig::new()
        .set_target_width(RENDER_TARGET_WIDTH)
        .set_maximum_height(RENDER_MAX_HEIGHT);

    let mut captures = Vec::with_capacity(pages.len());

    for &page_num in pages {
        let page_index = page_num.saturating_sub(1); // PDFium uses 0-based indices
        let page = doc
            .pages()
            .get(u16::try_from(page_index).unwrap_or(u16::MAX))
            .map_err(|err| {
                AppError::Processing(format!("failed to get PDF page {page_num}: {err}"))
            })?;

        let bitmap = page.render_with_config(&render_config).map_err(|err| {
            AppError::Processing(format!("failed to render PDF page {page_num}: {err}"))
        })?;

        let image = bitmap.as_image();

        let mut png_bytes = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut png_bytes), ImageFormat::Png)
            .map_err(|err| {
                AppError::Processing(format!(
                    "failed to encode PDF page {page_num} as PNG: {err}"
                ))
            })?;

        debug!(
            page = page_num,
            bytes = png_bytes.len(),
            "Rendered PDF page via PDFium"
        );

        if png_bytes.len() < MIN_PAGE_IMAGE_BYTES {
            warn!(
                page = page_num,
                bytes = png_bytes.len(),
                "Rendered page size below threshold; check PDF quality"
            );
        }

        captures.push(png_bytes);
    }

    Ok(captures)
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
    use lopdf::dictionary;
    use lopdf::Object;

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

    #[tokio::test]
    async fn test_load_page_numbers_empty_pdf() -> anyhow::Result<()> {
        let pdf_bytes = create_minimal_pdf(0);
        let pages = load_page_numbers(pdf_bytes).await?;
        assert!(pages.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_load_page_numbers_single_page() -> anyhow::Result<()> {
        let pdf_bytes = create_minimal_pdf(1);
        let pages = load_page_numbers(pdf_bytes).await?;
        assert_eq!(pages, vec![1u32]);
        Ok(())
    }

    #[tokio::test]
    async fn test_load_page_numbers_multi_page() -> anyhow::Result<()> {
        let pdf_bytes = create_minimal_pdf(5);
        let pages = load_page_numbers(pdf_bytes).await?;
        assert_eq!(pages, vec![1, 2, 3, 4, 5]);
        Ok(())
    }

    #[tokio::test]
    async fn test_load_page_numbers_invalid_pdf() {
        let result = load_page_numbers(b"not a pdf".to_vec()).await;
        assert!(result.is_err());
    }

    /// Creates a minimal valid PDF with the given number of empty pages.
    #[allow(clippy::similar_names, clippy::expect_used)]
    fn create_minimal_pdf(page_count: u32) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let mut page_ids = Vec::with_capacity(page_count as usize);
        for _ in 0..page_count {
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            page_ids.push(page_id);
        }

        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(),
            "Count" => i32::try_from(page_count).unwrap_or(i32::MAX),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut buf = Vec::new();
        doc.save_to(&mut buf).expect("failed to serialize test PDF");
        buf
    }

    /// Renders a simple 1-page PDF and verifies the output is a valid PNG ≥ 1KB.
    /// This test skips gracefully when `PDFium` is not available (e.g., CI without internet).
    #[tokio::test]
    async fn test_render_single_page_pdfium() -> anyhow::Result<()> {
        let pdf_bytes = create_minimal_pdf(1);
        let dir = tempfile::TempDir::new()?;
        let file_path = dir.path().join("test.pdf");
        tokio::fs::write(&file_path, &pdf_bytes).await?;

        let result = render_pdf_pages(&file_path, &[1]).await;
        match result {
            Ok(pages) => {
                assert_eq!(pages.len(), 1, "should render one page");
                #[allow(clippy::expect_used)]
                let first_page = pages.into_iter().next().expect("already asserted len == 1");
                assert!(
                    first_page.len() >= MIN_PAGE_IMAGE_BYTES,
                    "rendered page {} bytes is below threshold {}",
                    first_page.len(),
                    MIN_PAGE_IMAGE_BYTES
                );
                // Verify it's a valid PNG by checking header bytes
                let header = first_page
                    .get(..4.min(first_page.len()))
                    .unwrap_or(&[0u8; 0]);
                assert_eq!(header, &[0x89, 0x50, 0x4E, 0x47], "output must be PNG");
            }
            Err(e) => {
                // PDFium may not be available — that's acceptable in environments
                // without network access to download the binary.
                let msg = e.to_string();
                if !msg.contains("PDFium") && !msg.contains("library") && !msg.contains("bind") {
                    anyhow::bail!("unexpected error: {e}");
                }
                eprintln!("SKIP: PDFium not available ({msg})");
            }
        }
        Ok(())
    }
}
