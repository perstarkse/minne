pub mod llm_enrichment_result;

use std::{sync::Arc, time::Duration};

use async_openai::types::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs,
};
use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo, ingestion_payload::IngestionPayload, text_content::TextContent,
    },
};
use reqwest;
use scraper::{Html, Selector};
use std::fmt::Write;
use tiktoken_rs::{o200k_base, CoreBPE};

pub async fn to_text_content(
    ingestion_payload: IngestionPayload,
    openai_client: &Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
) -> Result<TextContent, AppError> {
    match ingestion_payload {
        IngestionPayload::Url {
            url,
            instructions,
            category,
            user_id,
        } => {
            let text = fetch_text_from_url(&url, openai_client).await?;
            Ok(TextContent::new(
                text,
                instructions.into(),
                category.into(),
                None,
                Some(url.into()),
                user_id.into(),
            ))
        }
        IngestionPayload::Text {
            text,
            instructions,
            category,
            user_id,
        } => Ok(TextContent::new(
            text.into(),
            instructions.into(),
            category.into(),
            None,
            None,
            user_id.into(),
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
                instructions.into(),
                category.into(),
                Some(file_info.to_owned()),
                None,
                user_id.into(),
            ))
        }
    }
}

/// Get text from url, will return it as a markdown formatted string
async fn fetch_text_from_url(
    url: &str,
    openai_client: &Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
) -> Result<String, AppError> {
    // Use a client with timeouts and reuse
    let client = reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = client.get(url).send().await?.text().await?;

    // Preallocate string with capacity
    let mut structured_content = String::with_capacity(response.len() / 2);

    let document = Html::parse_document(&response);
    let main_selectors = Selector::parse(
        "article, main, .article-content, .post-content, .entry-content, [role='main']",
    )
    .unwrap();

    let content_element = document
        .select(&main_selectors)
        .next()
        .or_else(|| document.select(&Selector::parse("body").unwrap()).next())
        .ok_or(AppError::NotFound("No content found".into()))?;

    // Compile selectors once
    let heading_selector = Selector::parse("h1, h2, h3").unwrap();
    let paragraph_selector = Selector::parse("p").unwrap();

    // Process content in one pass
    for element in content_element.select(&heading_selector) {
        let _ = writeln!(
            structured_content,
            "<heading>{}</heading>",
            element.text().collect::<String>().trim()
        );
    }
    for element in content_element.select(&paragraph_selector) {
        let _ = writeln!(
            structured_content,
            "<paragraph>{}</paragraph>",
            element.text().collect::<String>().trim()
        );
    }

    let content = structured_content
        .replace(|c: char| c.is_control(), " ")
        .replace("  ", " ");
    process_web_content(content, openai_client).await
}

pub async fn process_web_content(
    content: String,
    openai_client: &Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
) -> Result<String, AppError> {
    const MAX_TOKENS: usize = 122000;
    const SYSTEM_PROMPT: &str = r#"
        You are a precise content extractor for web pages. Your task:

        1. Extract ONLY the main article/content from the provided text
        2. Maintain the original content - do not summarize or modify the core information
        3. Ignore peripheral content such as:
            - Navigation elements
            - Error messages (e.g., "JavaScript required")
            - Related articles sections
            - Comments
            - Social media links
            - Advertisement text

        FORMAT:
        - Convert <heading> tags to markdown headings (#, ##, ###)
        - Convert <paragraph> tags to markdown paragraphs
        - Preserve quotes and important formatting
        - Remove duplicate content
        - Remove any metadata or technical artifacts

        OUTPUT RULES:
        - Output ONLY the cleaned content in markdown
        - Do not add any explanations or meta-commentary
        - Do not add summaries or conclusions
        - Do not use any XML/HTML tags in the output
    "#;

    let bpe = o200k_base()?;

    // Process content in chunks if needed
    let truncated_content = if bpe.encode_with_special_tokens(&content).len() > MAX_TOKENS {
        truncate_content(&content, MAX_TOKENS, &bpe)?
    } else {
        content
    };

    let request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .temperature(0.0)
        .max_tokens(16200u32)
        .messages([
            ChatCompletionRequestSystemMessage::from(SYSTEM_PROMPT).into(),
            ChatCompletionRequestUserMessage::from(truncated_content).into(),
        ])
        .build()?;

    let response = openai_client.chat().create(request).await?;

    response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .map(|content| content.to_owned())
        .ok_or(AppError::LLMParsing("No content in response".into()))
}

fn truncate_content(
    content: &str,
    max_tokens: usize,
    tokenizer: &CoreBPE,
) -> Result<String, AppError> {
    // Pre-allocate with estimated size
    let mut result = String::with_capacity(content.len() / 2);
    let mut current_tokens = 0;

    // Process content by paragraph to maintain context
    for paragraph in content.split("\n\n") {
        let tokens = tokenizer.encode_with_special_tokens(paragraph).len();

        // Check if adding paragraph exceeds limit
        if current_tokens + tokens > max_tokens {
            break;
        }

        result.push_str(paragraph);
        result.push_str("\n\n");
        current_tokens += tokens;
    }

    // Ensure we return valid content
    if result.is_empty() {
        return Err(AppError::Processing("Content exceeds token limit".into()));
    }

    Ok(result.trim_end().to_string())
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
