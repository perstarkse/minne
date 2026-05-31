mod render;
mod text;
mod vision;

use std::path::Path;

use common::{error::AppError, storage::db::SurrealDbClient, utils::config::PdfIngestMode};

use self::{
    render::{load_page_numbers, render_pdf_pages},
    text::{post_process, try_fast_path},
    vision::vision_markdown,
};

/// Upper bound on the number of pages handed to the vision model in a single document.
const MAX_VISION_PAGES: usize = 50;

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
