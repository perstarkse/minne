use uuid::Uuid;

use crate::models::file_info::FileInfo;
use crate::stored_entity;

stored_entity!(TextContent, "text_content", {
    text: String,
    file_info: Option<FileInfo>,
    instructions: String,
    category: String
});

impl TextContent {
    pub fn new(text: String, instructions: String, category: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            text,
            file_info: None,
            instructions,
            category,
        }
    }

    // Other methods...
}

fn test() {
    let content = TextContent::new(
        "hiho".to_string(),
        "instructions".to_string(),
        "cat".to_string(),
    );

    content.get_id();
}
