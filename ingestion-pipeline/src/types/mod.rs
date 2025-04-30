pub mod llm_enrichment_result;

use std::io::Write;
use std::time::Instant;

use axum::http::HeaderMap;
use axum_typed_multipart::{FieldData, FieldMetadata};
use chrono::Utc;
use common::storage::db::SurrealDbClient;
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo,
        ingestion_payload::IngestionPayload,
        text_content::{TextContent, UrlInfo},
    },
};
use dom_smoothie::{Article, Readability, TextMode};
use headless_chrome::Browser;
use tempfile::NamedTempFile;
use tracing::{error, info};

pub async fn to_text_content(
    ingestion_payload: IngestionPayload,
    db: &SurrealDbClient,
) -> Result<TextContent, AppError> {
    match ingestion_payload {
        IngestionPayload::Url {
            url,
            instructions,
            category,
            user_id,
        } => {
            let (article, file_info) = fetch_article_from_url(&url, db, &user_id).await?;
            Ok(TextContent::new(
                article.text_content.into(),
                instructions,
                category,
                None,
                Some(UrlInfo {
                    url,
                    title: article.title,
                    image_id: file_info.id,
                }),
                user_id,
            ))
        }
        IngestionPayload::Text {
            text,
            instructions,
            category,
            user_id,
        } => Ok(TextContent::new(
            text,
            instructions,
            category,
            None,
            None,
            user_id,
        )),
        IngestionPayload::File {
            file_info,
            instructions,
            category,
            user_id,
        } => {
            let text = extract_text_from_file(&file_info).await?;
            Ok(TextContent::new(
                text,
                instructions,
                category,
                Some(file_info),
                None,
                user_id,
            ))
        }
    }
}
use std::io::{Seek, SeekFrom}; // <-- Add Seek and SeekFrom

/// Fetches web content from a URL, extracts the main article text as Markdown,
/// captures a screenshot, and stores the screenshot returning [`FileInfo`].
///
/// This function handles browser automation, content extraction via Readability,
/// screenshot capture, temporary file handling, and persisting the screenshot
/// details (including deduplication based on content hash via [`FileInfo::new`]).
///
/// # Arguments
///
/// * `url` - The URL of the web page to fetch.
/// * `db` - A reference to the database client (`SurrealDbClient`).
/// * `user_id` - The ID of the user performing the action, used for associating the file.
///
/// # Returns
///
/// A `Result` containing:
/// * Ok: A tuple `(Article, FileInfo)` where `Article` contains the parsed markdown
///   content and metadata, and `FileInfo` contains the details of the stored screenshot.
/// * Err: An `AppError` if any step fails (navigation, screenshot, file handling, DB operation).
async fn fetch_article_from_url(
    url: &str,
    db: &SurrealDbClient,
    user_id: &str,
) -> Result<(Article, FileInfo), AppError> {
    info!("Fetching URL: {}", url);
    // Instantiate timer
    let now = Instant::now();
    // Setup browser, navigate and wait
    let browser = Browser::default()?;
    let tab = browser.new_tab()?;
    let page = tab.navigate_to(url)?;
    let loaded_page = page.wait_until_navigated()?;
    // Get content
    let raw_content = loaded_page.get_content()?;
    // Get screenshot
    let screenshot = loaded_page.capture_screenshot(
        headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Jpeg,
        None,
        None,
        true,
    )?;

    // Create temp file
    let mut tmp_file = NamedTempFile::new()?;
    let temp_path_str = format!("{:?}", tmp_file.path());

    // Write screenshot TO the temp file
    tmp_file.write_all(&screenshot)?;

    // Ensure the OS buffer is written to the file system _before_ we proceed.
    tmp_file.as_file().sync_all()?;

    // Ensure the file handle's read cursor is at the beginning before hashing occurs.
    if let Err(e) = tmp_file.seek(SeekFrom::Start(0)) {
        error!("URL: {}. Failed to seek temp file {} to start: {:?}. Proceeding, but hashing might fail.", url, temp_path_str, e);
    }

    // Prepare file metadata
    let parsed_url =
        url::Url::parse(url).map_err(|_| AppError::Processing("Invalid URL".to_string()))?;
    let domain = parsed_url
        .host_str()
        .unwrap_or("unknown")
        .replace(|c: char| !c.is_alphanumeric(), "_");
    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let file_name = format!("{}_{}_{}.jpg", domain, "screenshot", timestamp);

    // Construct FieldData and FieldMetadata
    let metadata = FieldMetadata {
        file_name: Some(file_name),
        content_type: Some("image/jpeg".to_string()),
        name: None,
        headers: HeaderMap::new(),
    };
    let field_data = FieldData {
        contents: tmp_file,
        metadata,
    };

    // Store screenshot
    let file_info = FileInfo::new(field_data, db, user_id).await?;

    // Parse content...
    let config = dom_smoothie::Config {
        text_mode: TextMode::Markdown,
        ..Default::default()
    };
    let mut readability = Readability::new(raw_content, None, Some(config))?;
    let article: Article = readability.parse()?;
    let end = now.elapsed();
    info!(
        "URL: {}. Total time: {:?}. Final File ID: {}",
        url, end, file_info.id
    );

    Ok((article, file_info))
}

/// Extracts text from a file based on its MIME type.
async fn extract_text_from_file(file_info: &FileInfo) -> Result<String, AppError> {
    match file_info.mime_type.as_str() {
        "text/plain" => {
            // Read the file and return its content
            let content = tokio::fs::read_to_string(&file_info.path).await?;
            Ok(content)
        }
        "text/markdown" => {
            // Read the file and return its content
            let content = tokio::fs::read_to_string(&file_info.path).await?;
            Ok(content)
        }
        "application/pdf" => {
            // TODO: Implement PDF text extraction using a crate like `pdf-extract` or `lopdf`
            Err(AppError::NotFound(file_info.mime_type.clone()))
        }
        "image/png" | "image/jpeg" => {
            // TODO: Implement OCR on image using a crate like `tesseract`
            Err(AppError::NotFound(file_info.mime_type.clone()))
        }
        "application/octet-stream" => {
            let content = tokio::fs::read_to_string(&file_info.path).await?;
            Ok(content)
        }
        "text/x-rust" => {
            let content = tokio::fs::read_to_string(&file_info.path).await?;
            Ok(content)
        }
        // Handle other MIME types as needed
        _ => Err(AppError::NotFound(file_info.mime_type.clone())),
    }
}
