use crate::{
    error::AppError,
    storage::types::{file_info::FileInfo, text_content::TextContent},
};
use async_openai::types::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs,
};
use reqwest;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tiktoken_rs::o200k_base;
use tracing::info;

/// Knowledge object type, containing the content or reference to it, as well as metadata
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum IngressObject {
    Url {
        url: String,
        instructions: String,
        category: String,
        user_id: String,
    },
    Text {
        text: String,
        instructions: String,
        category: String,
        user_id: String,
    },
    File {
        file_info: FileInfo,
        instructions: String,
        category: String,
        user_id: String,
    },
}

impl IngressObject {
    /// Creates a new `TextContent` instance from a `IngressObject`.
    ///
    /// # Arguments
    /// `&self` - A reference to the `IngressObject`.
    ///
    /// # Returns
    /// `TextContent` - An object containing a text representation of the object, could be a scraped URL, parsed PDF, etc.
    pub async fn to_text_content(&self) -> Result<TextContent, AppError> {
        match self {
            IngressObject::Url {
                url,
                instructions,
                category,
                user_id,
            } => {
                let text = Self::fetch_text_from_url(url).await?;
                Ok(TextContent::new(
                    text,
                    instructions.into(),
                    category.into(),
                    None,
                    user_id.into(),
                ))
            }
            IngressObject::Text {
                text,
                instructions,
                category,
                user_id,
            } => Ok(TextContent::new(
                text.into(),
                instructions.into(),
                category.into(),
                None,
                user_id.into(),
            )),
            IngressObject::File {
                file_info,
                instructions,
                category,
                user_id,
            } => {
                let text = Self::extract_text_from_file(file_info).await?;
                Ok(TextContent::new(
                    text,
                    instructions.into(),
                    category.into(),
                    Some(file_info.to_owned()),
                    user_id.into(),
                ))
            }
        }
    }

    /// Fetches and extracts text from a URL.
    async fn fetch_text_from_url(url: &str) -> Result<String, AppError> {
        let response = reqwest::get(url).await?.text().await?;
        let document = Html::parse_document(&response);

        // Select main content areas first
        let main_selectors = Selector::parse(concat!(
            "article, main, .article-content,", // Common main content classes
            ".post-content, .entry-content,",   // Common blog/article classes
            "[role='main']"                     // Accessibility marker
        ))
        .unwrap();

        // If no main content found, fallback to body
        let content_element = document
            .select(&main_selectors)
            .next()
            .or_else(|| document.select(&Selector::parse("body").unwrap()).next())
            .ok_or(AppError::NotFound("No content found".into()))?;

        // Remove unwanted elements but preserve structure
        // let exclude_selector = Selector::parse(concat!(
        //     "script, style, noscript,",
        //     "[class*='window'], [id*='window'],",
        //     "[class*='env'], [id*='env'],",
        //     "iframe, nav, footer, .comments,",
        //     ".advertisement, .social-share"
        // ))
        // .unwrap();

        // Collect structured content
        let mut structured_content = String::new();

        // Process headings
        for heading in content_element.select(&Selector::parse("h1, h2, h3").unwrap()) {
            structured_content.push_str(&format!(
                "<heading>{}</heading>\n",
                heading.text().collect::<String>().trim()
            ));
        }

        // Process paragraphs
        for paragraph in content_element.select(&Selector::parse("p").unwrap()) {
            structured_content.push_str(&format!(
                "<paragraph>{}</paragraph>\n",
                paragraph.text().collect::<String>().trim()
            ));
        }

        // Clean up
        let content = structured_content
            .replace(|c: char| c.is_control(), " ")
            .replace("  ", " ");

        let processed_content = Self::process_web_content(content.trim().to_string()).await?;

        info!("Extracted content from page: {:?}", processed_content);

        Ok(processed_content)
    }

    pub async fn process_web_content(content: String) -> Result<String, AppError> {
        let openai_client = async_openai::Client::new();
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
        let token_count = bpe.encode_with_special_tokens(&content).len();

        let content = if token_count > MAX_TOKENS {
            // Split content into structural blocks
            let blocks: Vec<&str> = content.split('\n').collect();
            let mut truncated = String::new();
            let mut current_tokens = 0;

            // Keep adding blocks until we approach the limit
            for block in blocks {
                let block_tokens = bpe.encode_with_special_tokens(block).len();
                if current_tokens + block_tokens > MAX_TOKENS {
                    break;
                }
                truncated.push_str(block);
                truncated.push('\n');
                current_tokens += block_tokens;
            }
            truncated
        } else {
            content
        };

        let request = CreateChatCompletionRequestArgs::default()
            .model("gpt-4o-mini")
            .temperature(0.0)
            .max_tokens(16200u32)
            .messages([
                ChatCompletionRequestSystemMessage::from(SYSTEM_PROMPT).into(),
                ChatCompletionRequestUserMessage::from(content).into(),
            ])
            .build()?;

        let response = openai_client.chat().create(request).await?;

        response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .map(|content| content.to_string())
            .ok_or(AppError::LLMParsing("No content in response".into()))
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
}
