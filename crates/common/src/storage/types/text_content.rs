use surrealdb::opt::PatchOp;
use uuid::Uuid;

use crate::{error::AppError, storage::db::SurrealDbClient, stored_object};

use super::file_info::FileInfo;

stored_object!(TextContent, "text_content", {
    text: String,
    file_info: Option<FileInfo>,
    url: Option<String>,
    instructions: String,
    category: String,
    user_id: String
});

impl TextContent {
    pub fn new(
        text: String,
        instructions: String,
        category: String,
        file_info: Option<FileInfo>,
        url: Option<String>,
        user_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            text,
            file_info,
            url,
            instructions,
            category,
            user_id,
        }
    }

    pub async fn patch(
        id: &str,
        instructions: &str,
        category: &str,
        text: &str,
        db: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let now = Utc::now();

        let _res: Option<Self> = db
            .update((Self::table_name(), id))
            .patch(PatchOp::replace("/instructions", instructions))
            .patch(PatchOp::replace("/category", category))
            .patch(PatchOp::replace("/text", text))
            .patch(PatchOp::replace("/updated_at", now))
            .await?;

        Ok(())
    }
}
