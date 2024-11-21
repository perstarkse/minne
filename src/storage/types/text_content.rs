use uuid::Uuid;

use crate::models::file_info::FileInfo;
use crate::stored_object;

stored_object!(TextContent, "text_content", {
    text: String,
    file_info: Option<FileInfo>,
    instructions: String,
    category: String
});

impl TextContent {
    pub fn new(
        text: String,
        instructions: String,
        category: String,
        file_info: Option<FileInfo>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            text,
            file_info,
            instructions,
            category,
        }
    }
}
