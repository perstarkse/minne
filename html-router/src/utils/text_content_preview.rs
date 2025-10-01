use common::storage::types::text_content::TextContent;

const TEXT_PREVIEW_LENGTH: usize = 50;

fn maybe_truncate(value: &str) -> Option<String> {
    for (char_count, (idx, _)) in value.char_indices().enumerate() {
        if char_count == TEXT_PREVIEW_LENGTH {
            return Some(value[..idx].to_string());
        }
    }

    None
}

pub fn truncate_text_content(mut content: TextContent) -> TextContent {
    if let Some(truncated) = maybe_truncate(&content.text) {
        content.text = truncated;
    }

    if let Some(context) = content.context.as_mut() {
        if let Some(truncated) = maybe_truncate(context) {
            *context = truncated;
        }
    }

    content
}

pub fn truncate_text_contents(contents: Vec<TextContent>) -> Vec<TextContent> {
    contents.into_iter().map(truncate_text_content).collect()
}
