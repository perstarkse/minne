//! Vision-LLM transcription of rendered PDF pages into Markdown.

use async_openai::types::chat::{
    ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequest, CreateChatCompletionRequestArgs, ImageDetail, ImageUrlArgs,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tracing::{debug, warn};

use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};

const PAGES_PER_VISION_CHUNK: usize = 4;
const MAX_VISION_ATTEMPTS: usize = 2;
const PDF_MARKDOWN_PROMPT: &str = "Convert these PDF pages to clean Markdown. Preserve headings, lists, tables, blockquotes, code fences, and inline formatting. Keep the original reading order, avoid commentary, and do NOT wrap the entire response in a Markdown code block.";
const PDF_MARKDOWN_PROMPT_RETRY: &str = "You must transcribe the provided PDF page images into accurate Markdown. The images are already supplied, so do not respond that you cannot view them. Extract all visible text, tables, and structure, and do NOT wrap the overall response in a Markdown code block.";

/// Sends rendered pages to the configured multimodal model in batches and stitches the
/// resulting Markdown chunks together.
pub(super) async fn vision_markdown(
    rendered_pages: Vec<Vec<u8>>,
    db: &SurrealDbClient,
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<String, AppError> {
    let settings = SystemSettings::get_current(db).await?;
    let model = settings.image_processing_model;

    debug!(
        pages = rendered_pages.len(),
        "Preparing vision batches for PDF conversion"
    );

    let mut markdown_sections = Vec::with_capacity(rendered_pages.len());

    for (batch_idx, chunk) in rendered_pages.chunks(PAGES_PER_VISION_CHUNK).enumerate() {
        let encoded_images = encode_batch(batch_idx, chunk);
        let markdown = transcribe_batch(client, &model, batch_idx, &encoded_images).await?;
        markdown_sections.push(markdown);
    }

    Ok(markdown_sections.join("\n\n"))
}

/// Base64-encodes one batch of page images, warning on suspiciously tiny payloads.
fn encode_batch(batch_idx: usize, chunk: &[Vec<u8>]) -> Vec<String> {
    let total_image_bytes: usize = chunk.iter().map(Vec::len).sum();
    debug!(
        batch = batch_idx,
        pages = chunk.len(),
        bytes = total_image_bytes,
        "Encoding PDF images for vision batch"
    );

    chunk
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
        .collect()
}

/// Requests Markdown for a single batch, retrying with a stronger prompt on low-quality output.
async fn transcribe_batch(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    model: &str,
    batch_idx: usize,
    encoded_images: &[String],
) -> Result<String, AppError> {
    let last_attempt = MAX_VISION_ATTEMPTS.saturating_sub(1);

    for attempt in 0..MAX_VISION_ATTEMPTS {
        let request = build_request(model, prompt_for_attempt(attempt), encoded_images)?;

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
        log_preview(batch_idx, attempt, content);

        if is_low_quality_response(content) {
            warn!(
                batch = batch_idx,
                attempt, "Vision model returned low quality response"
            );
            if attempt == last_attempt {
                return Err(AppError::Processing(
                    "vision model failed to transcribe PDF page contents".into(),
                ));
            }
            continue;
        }

        return Ok(content.trim().to_string());
    }

    Err(AppError::Processing(
        "vision model did not return usable Markdown".into(),
    ))
}

/// Builds the chat-completion request carrying the prompt and the batch's images.
fn build_request(
    model: &str,
    prompt_text: &str,
    encoded_images: &[String],
) -> Result<CreateChatCompletionRequest, AppError> {
    let mut content_parts = Vec::with_capacity(encoded_images.len().saturating_add(1));
    content_parts.push(
        ChatCompletionRequestMessageContentPartTextArgs::default()
            .text(prompt_text)
            .build()?
            .into(),
    );

    for encoded in encoded_images {
        let image_url = format!("data:image/png;base64,{encoded}");
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
        .model(model)
        .messages([ChatCompletionRequestUserMessageArgs::default()
            .content(content_parts)
            .build()?
            .into()])
        .build()?;

    Ok(request)
}

/// Logs a truncated preview of a model response at debug level.
fn log_preview(batch_idx: usize, attempt: usize, content: &str) {
    let preview: String = if content.len() > 500 {
        let mut snippet = content.chars().take(500).collect::<String>();
        snippet.push('…');
        snippet
    } else {
        content.to_string()
    };
    debug!(batch = batch_idx, attempt, preview = %preview, "Vision response content preview");
}

fn is_low_quality_response(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lowered = trimmed.to_ascii_lowercase();
    lowered.contains("unable to") || lowered.contains("cannot")
}

const fn prompt_for_attempt(attempt: usize) -> &'static str {
    if attempt == 0 {
        PDF_MARKDOWN_PROMPT
    } else {
        PDF_MARKDOWN_PROMPT_RETRY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_low_quality_response_detection() {
        assert!(is_low_quality_response(""));
        assert!(is_low_quality_response("I'm unable to help."));
        assert!(is_low_quality_response("I cannot read this."));
        assert!(!is_low_quality_response("# Heading\nValid content"));
    }

    #[test]
    fn test_prompt_for_attempt_variants() {
        assert_eq!(prompt_for_attempt(0), PDF_MARKDOWN_PROMPT);
        assert_eq!(prompt_for_attempt(1), PDF_MARKDOWN_PROMPT_RETRY);
        assert_eq!(prompt_for_attempt(5), PDF_MARKDOWN_PROMPT_RETRY);
    }

    #[test]
    fn test_markdown_prompts_discourage_code_blocks() {
        assert!(!PDF_MARKDOWN_PROMPT.contains("```"));
        assert!(!PDF_MARKDOWN_PROMPT_RETRY.contains("```"));
    }
}
