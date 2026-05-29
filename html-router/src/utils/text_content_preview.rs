use common::storage::types::text_content::TextContent;

use super::truncate::with_ellipsis;

const TEXT_PREVIEW_LENGTH: usize = 50;

pub fn truncate_text_content(mut content: TextContent) -> TextContent {
    content.text = with_ellipsis(&content.text, TEXT_PREVIEW_LENGTH);

    if let Some(context) = content.context.as_mut() {
        *context = with_ellipsis(context, TEXT_PREVIEW_LENGTH);
    }

    content
}

pub fn truncate_text_contents(contents: Vec<TextContent>) -> Vec<TextContent> {
    contents.into_iter().map(truncate_text_content).collect()
}
