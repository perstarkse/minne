use axum::http::HeaderMap;
use axum_typed_multipart::{FieldData, FieldMetadata};
use chrono::Utc;
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, store::StorageManager, types::file_info::FileInfo},
};
use dom_smoothie::Article;
use std::{
    io::{Seek, SeekFrom, Write},
    net::IpAddr,
    time::Instant,
};
use tempfile::NamedTempFile;
use tendril::StrTendril;
use tracing::{info, warn};

use crate::utils::page_fetcher::create_fetcher;

pub async fn extract_text_from_url(
    url: &str,
    db: &SurrealDbClient,
    user_id: &str,
    storage: &StorageManager,
) -> Result<(Article, FileInfo), AppError> {
    info!("Fetching URL: {}", url);
    let now = Instant::now();

    let parsed_url =
        url::Url::parse(url).map_err(|_| AppError::Validation("invalid URL".to_string()))?;
    let domain = ensure_ingestion_url_allowed(&parsed_url)?;

    let fetcher = create_fetcher();
    let capture = fetcher.fetch(url)?;

    // Save the screenshot to storage
    let mut tmp_file = NamedTempFile::new()?;

    if !capture.screenshot.is_empty() {
        tmp_file.write_all(&capture.screenshot)?;
        tmp_file.as_file().sync_all()?;
        tmp_file.seek(SeekFrom::Start(0))?;
    }

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let file_name = format!("{}_{}_{}.jpg", domain, "screenshot", timestamp);

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

    let file_info = FileInfo::new_with_storage(field_data, db, user_id, storage).await?;

    // servo-fetch doesn't extract byline/site_name/metadata, so those are left empty.
    let title = extract_title_from_html(&capture.html);
    let article = Article {
        title,
        byline: None,
        content: StrTendril::from_slice(&capture.markdown),
        text_content: StrTendril::from_slice(&capture.markdown),
        length: capture.markdown.len(),
        excerpt: None,
        site_name: None,
        dir: None,
        lang: None,
        published_time: None,
        modified_time: None,
        image: None,
        favicon: None,
        url: Some(url.to_string()),
    };

    let end = now.elapsed();
    info!(
        "URL: {}. Total time: {:?}. Final File ID: {}",
        url, end, file_info.id
    );

    Ok((article, file_info))
}

/// Extracts a page title from raw HTML. Returns empty string when no title is found.
fn extract_title_from_html(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if let Some(start) = lower.find("<title>") {
        let content_start = start.saturating_add("<title>".len());
        if let Some(end) = lower[content_start..].find("</title>") {
            let title_end = content_start.saturating_add(end);
            if title_end <= html.len() {
                let title = html[content_start..title_end].trim().to_string();
                if !title.is_empty() {
                    return title;
                }
            }
        }
    }
    String::new()
}

fn ensure_ingestion_url_allowed(url: &url::Url) -> Result<String, AppError> {
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            warn!(%url, %scheme, "Rejected ingestion URL due to unsupported scheme");
            return Err(AppError::Validation(
                "unsupported URL scheme for ingestion".to_string(),
            ));
        }
    }

    let Some(host) = url.host_str() else {
        warn!(%url, "Rejected ingestion URL missing host");
        return Err(AppError::Validation(
            "URL missing a host component".to_string(),
        ));
    };

    if host.eq_ignore_ascii_case("localhost") {
        warn!(%url, host, "Rejected ingestion URL to localhost");
        return Err(AppError::Validation(
            "ingestion URL host is not allowed".to_string(),
        ));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        let is_disallowed = match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_unique_local() || v6.is_unicast_link_local(),
        };

        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || is_disallowed {
            warn!(%url, host, %ip, "Rejected ingestion URL pointing to restricted network range");
            return Err(AppError::Validation(
                "ingestion URL host is not allowed".to_string(),
            ));
        }
    }

    Ok(host.replace(|c: char| !c.is_alphanumeric(), "_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{self};

    #[test]
    fn rejects_unsupported_scheme() -> anyhow::Result<()> {
        let url = url::Url::parse("ftp://example.com")?;
        assert!(ensure_ingestion_url_allowed(&url).is_err());
        Ok(())
    }

    #[test]
    fn rejects_localhost() -> anyhow::Result<()> {
        let url = url::Url::parse("http://localhost/resource")?;
        assert!(ensure_ingestion_url_allowed(&url).is_err());
        Ok(())
    }

    #[test]
    fn rejects_private_ipv4() -> anyhow::Result<()> {
        let url = url::Url::parse("http://192.168.1.10/index.html")?;
        assert!(ensure_ingestion_url_allowed(&url).is_err());
        Ok(())
    }

    #[test]
    fn allows_public_domain_and_sanitizes() -> anyhow::Result<()> {
        let url = url::Url::parse("https://sub.example.com/path")?;
        let sanitized = ensure_ingestion_url_allowed(&url)?;
        assert_eq!(sanitized, "sub_example_com");
        Ok(())
    }

    #[test]
    fn test_extract_title_from_html_with_title() {
        let html = "<html><head><title>Hello World</title></head><body></body></html>";
        assert_eq!(extract_title_from_html(html), "Hello World");
    }

    #[test]
    fn test_extract_title_from_html_mixed_case() {
        let html = "<html><head><TITLE>Mixed Case</TITLE></head><body></body></html>";
        assert_eq!(extract_title_from_html(html), "Mixed Case");
    }

    #[test]
    fn test_extract_title_from_html_no_title() {
        let html = "<html><head></head><body><p>No title here</p></body></html>";
        assert_eq!(extract_title_from_html(html), "");
    }

    #[test]
    fn test_extract_title_from_html_empty_title() {
        let html = "<html><head><title></title></head><body></body></html>";
        assert_eq!(extract_title_from_html(html), "");
    }
}
