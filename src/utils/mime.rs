use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use url::Url;
use uuid::Uuid;
use std::path::Path;
use tokio::fs;


/// Struct to reference stored files.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Reference {
    pub uuid: Uuid,
    pub path: String,
}

impl Reference {
    /// Creates a new Reference with a generated UUID.
    pub fn new(path: String) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            path,
        }
    }
}

/// Enum representing different types of content.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Content {
    Text(String),
    Url(String),
    Document(Reference),
    Video(Reference),
    Audio(Reference),
    // Extend with more variants as needed
}

impl Content {
    /// Retrieves the path from a reference if the content is a Reference variant.
    pub fn get_path(&self) -> Option<&str> {
        match self {
            Content::Document(ref r) | Content::Video(ref r) | Content::Audio(ref r) => Some(&r.path),
            _ => None,
        }
    }
}
#[derive(Error, Debug)]
pub enum IngressContentError {
    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("MIME type detection failed for input: {0}")]
    MimeDetection(String),

    #[error("Unsupported MIME type: {0}")]
    UnsupportedMime(String),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    // Add more error variants as needed.
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IngressContent {
    pub content: Content,
    pub category: String,
    pub instructions: String,
}

impl IngressContent {
    /// Creates a new IngressContent instance from the given input.
    ///
    /// # Arguments
    ///
    /// * `input` - A string slice that holds the input content, which can be text, a file path, or a URL.
    /// * `category` - A string slice representing the category of the content.
    /// * `instructions` - A string slice containing instructions for processing the content.
    ///
    /// # Returns
    ///
    /// * `Result<IngressContent, IngressContentError>` - The result containing either the IngressContent instance or an error.
    pub async fn new(
        input: &str,
        category: &str,
        instructions: &str,
    ) -> Result<IngressContent, IngressContentError> {
        // Check if the input is a valid URL
        if let Ok(url) = Url::parse(input) {
            info!("Detected URL: {}", url);
            return Ok(IngressContent {
                content: Content::Url(url.to_string()),
                category: category.to_string(),
                instructions: instructions.to_string(),
            });
        }

        // Attempt to treat the input as a file path
        if let Ok(metadata) = tokio::fs::metadata(input).await {
            if metadata.is_file() {
                info!("Processing as file path: {}", input);
                let mime = mime_guess::from_path(input).first_or(mime::TEXT_PLAIN);
                let reference = Self::store_file(input, &mime).await?;
                let content = match mime.type_() {
                    mime::TEXT | mime::APPLICATION => Content::Document(reference),
                    mime::VIDEO => Content::Video(reference),
                    mime::AUDIO => Content::Audio(reference),
                    other => {
                        info!("Detected unsupported MIME type: {}", other);
                        return Err(IngressContentError::UnsupportedMime(mime.to_string()));
                    }
                };
                return Ok(IngressContent {
                    content,
                    category: category.to_string(),
                    instructions: instructions.to_string(),
                });
            }
        }

        // Treat the input as plain text if it's neither a URL nor a file path
        info!("Treating input as plain text");
        Ok(IngressContent {
            content: Content::Text(input.to_string()),
            category: category.to_string(),
            instructions: instructions.to_string(),
        })
    }

    /// Stores the file into 'data/' directory and returns a Reference.
    async fn store_file(input_path: &str, mime: &mime::Mime) -> Result<Reference, IngressContentError> {

        return Ok(Reference::new(input_path.to_string()));        

        // Define the data directory
        let data_dir = Path::new("data/");

        // Ensure 'data/' directory exists; create it if it doesn't
        fs::create_dir_all(data_dir).await.map_err(IngressContentError::Io)?;

        // Generate a UUID for the file
        let uuid = Uuid::new_v4();

        // Determine the file extension based on MIME type
        // let extension = Some(mime_guess::get_mime_extensions(mime)).unwrap_or("bin");

        // Create a unique filename using UUID and extension
        let file_name = format!("{}.{}", uuid, extension);

        // Define the full file path
        let file_path = data_dir.join(&file_name);

        // Copy the original file to the 'data/' directory with the new filename
        fs::copy(input_path, &file_path).await.map_err(IngressContentError::Io)?;

        // Return a new Reference
        Ok(Reference::new(file_path.to_string_lossy().to_string()))
    }

    /// Example method to handle content. Implement your actual logic here.
    pub fn handle_content(&self) {
        match &self.content {
            Content::Text(text) => {
                // Handle text content
                println!("Text: {}", text);
            }
            Content::Url(url) => {
                // Handle URL content
                println!("URL: {}", url);
            }
            Content::Document(ref reference) => {
                // Handle Document content via reference
                println!("Document Reference: UUID: {}, Path: {}", reference.uuid, reference.path);
                // Optionally, read the file from reference.path
            }
            Content::Video(ref reference) => {
                // Handle Video content via reference
                println!("Video Reference: UUID: {}, Path: {}", reference.uuid, reference.path);
                // Optionally, read the file from reference.path
            }
            Content::Audio(ref reference) => {
                // Handle Audio content via reference
                println!("Audio Reference: UUID: {}, Path: {}", reference.uuid, reference.path);
                // Optionally, read the file from reference.path
            }
            // Handle additional content types
        }
    }
}
