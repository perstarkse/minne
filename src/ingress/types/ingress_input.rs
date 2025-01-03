use super::ingress_object::IngressObject;
use crate::{
    error::AppError,
    storage::{
        db::{get_item, SurrealDbClient},
        types::file_info::FileInfo,
    },
};
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

/// Struct defining the expected body when ingressing content.
#[derive(Serialize, Deserialize, Debug)]
pub struct IngressInput {
    pub content: Option<String>,
    pub instructions: String,
    pub category: String,
    pub files: Vec<FileInfo>,
}

/// Function to create ingress objects from input.
///
/// # Arguments
/// * `input` - IngressInput containing information needed to ingress content.
/// * `user_id` - User id of the ingressing user
///
/// # Returns
/// * `Vec<IngressObject>` - An array containing the ingressed objects, one file/contenttype per object.
pub fn create_ingress_objects(
    input: IngressInput,
    user_id: &str,
) -> Result<Vec<IngressObject>, AppError> {
    // Initialize list
    let mut object_list = Vec::new();

    // Create a IngressObject from input.content if it exists, checking for URL or text
    if let Some(input_content) = input.content {
        match Url::parse(&input_content) {
            Ok(url) => {
                info!("Detected URL: {}", url);
                object_list.push(IngressObject::Url {
                    url: url.to_string(),
                    instructions: input.instructions.clone(),
                    category: input.category.clone(),
                    user_id: user_id.into(),
                });
            }
            Err(_) => {
                info!("Treating input as plain text");
                object_list.push(IngressObject::Text {
                    text: input_content.to_string(),
                    instructions: input.instructions.clone(),
                    category: input.category.clone(),
                    user_id: user_id.into(),
                });
            }
        }
    }

    for file in input.files {
        object_list.push(IngressObject::File {
            file_info: file,
            instructions: input.instructions.clone(),
            category: input.category.clone(),
            user_id: user_id.into(),
        })
    }

    // If no objects are constructed, we return Err
    if object_list.is_empty() {
        return Err(AppError::NotFound(
            "No valid content or files provided".into(),
        ));
    }

    Ok(object_list)
}
