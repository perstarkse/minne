use axum::http::HeaderMap;
use axum_typed_multipart::{FieldData, FieldMetadata};
use chrono::Utc;
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, store::StorageManager, types::file_info::FileInfo},
};
use dom_smoothie::{Article, Readability, TextMode};
use headless_chrome::Browser;
use std::{
    io::{Seek, SeekFrom, Write},
    net::IpAddr,
    time::Instant,
};
use tempfile::NamedTempFile;
use tracing::{error, info, warn};
pub async fn extract_text_from_url(
    url: &str,
    db: &SurrealDbClient,
    user_id: &str,
    storage: &StorageManager,
) -> Result<(Article, FileInfo), AppError> {
    info!("Fetching URL: {}", url);
    let now = Instant::now();

    let browser = {
        #[cfg(feature = "docker")]
        {
            let options = headless_chrome::LaunchOptionsBuilder::default()
                .sandbox(false)
                .build()
                .map_err(|e| AppError::InternalError(e.to_string()))?;
            Browser::new(options)?
        }
        #[cfg(not(feature = "docker"))]
        {
            Browser::default()?
        }
    };

    let tab = browser.new_tab()?;
    let page = tab.navigate_to(url)?;
    let loaded_page = page.wait_until_navigated()?;
    let raw_content = loaded_page.get_content()?;
    let screenshot = loaded_page.capture_screenshot(
        headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Jpeg,
        None,
        None,
        true,
    )?;

    let mut tmp_file = NamedTempFile::new()?;
    let temp_path_str = tmp_file.path().display().to_string();

    tmp_file.write_all(&screenshot)?;
    tmp_file.as_file().sync_all()?;

    if let Err(e) = tmp_file.seek(SeekFrom::Start(0)) {
        error!(
            "URL: {}. Failed to seek temp file {} to start: {:?}. Proceeding, but hashing might fail.",
            url, temp_path_str, e
        );
    }

    let parsed_url =
        url::Url::parse(url).map_err(|_| AppError::Validation("Invalid URL".to_string()))?;

    let domain = ensure_ingestion_url_allowed(&parsed_url)?;
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

fn ensure_ingestion_url_allowed(url: &url::Url) -> Result<String, AppError> {
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            warn!(%url, %scheme, "Rejected ingestion URL due to unsupported scheme");
            return Err(AppError::Validation(
                "Unsupported URL scheme for ingestion".to_string(),
            ));
        }
    }

    let Some(host) = url.host_str() else {
        warn!(%url, "Rejected ingestion URL missing host");
        return Err(AppError::Validation(
            "URL is missing a host component".to_string(),
        ));
    };

    if host.eq_ignore_ascii_case("localhost") {
        warn!(%url, host, "Rejected ingestion URL to localhost");
        return Err(AppError::Validation(
            "Ingestion URL host is not allowed".to_string(),
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
                "Ingestion URL host is not allowed".to_string(),
            ));
        }
    }

    Ok(host.replace(|c: char| !c.is_alphanumeric(), "_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_scheme() {
        let url = url::Url::parse("ftp://example.com").expect("url");
        assert!(ensure_ingestion_url_allowed(&url).is_err());
    }

    #[test]
    fn rejects_localhost() {
        let url = url::Url::parse("http://localhost/resource").expect("url");
        assert!(ensure_ingestion_url_allowed(&url).is_err());
    }

    #[test]
    fn rejects_private_ipv4() {
        let url = url::Url::parse("http://192.168.1.10/index.html").expect("url");
        assert!(ensure_ingestion_url_allowed(&url).is_err());
    }

    #[test]
    fn allows_public_domain_and_sanitizes() {
        let url = url::Url::parse("https://sub.example.com/path").expect("url");
        let sanitized = ensure_ingestion_url_allowed(&url).expect("allowed");
        assert_eq!(sanitized, "sub_example_com");
    }
}
