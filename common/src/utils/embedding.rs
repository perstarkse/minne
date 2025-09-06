use async_openai::types::CreateEmbeddingRequestArgs;
use tracing::debug;

use crate::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};
/// Generates an embedding vector for the given input text using OpenAI's embedding model.
///
/// This function takes a text input and converts it into a numerical vector representation (embedding)
/// using OpenAI's text-embedding-3-small model. These embeddings can be used for semantic similarity
/// comparisons, vector search, and other natural language processing tasks.
///
/// # Arguments
///
/// * `client`: The OpenAI client instance used to make API requests.
/// * `input`: The text string to generate embeddings for.
///
/// # Returns
///
/// Returns a `Result` containing either:
/// * `Ok(Vec<f32>)`: A vector of 32-bit floating point numbers representing the text embedding
/// * `Err(ProcessingError)`: An error if the embedding generation fails
///
/// # Errors
///
/// This function can return a `AppError` in the following cases:
/// * If the OpenAI API request fails
/// * If the request building fails
/// * If no embedding data is received in the response
pub async fn generate_embedding(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    input: &str,
    db: &SurrealDbClient,
) -> Result<Vec<f32>, AppError> {
    let model = SystemSettings::get_current(db).await?;

    let request = CreateEmbeddingRequestArgs::default()
        .model(model.embedding_model)
        .dimensions(model.embedding_dimensions)
        .input([input])
        .build()?;

    // Send the request to OpenAI
    let response = client.embeddings().create(request).await?;

    // Extract the embedding vector
    let embedding: Vec<f32> = response
        .data
        .first()
        .ok_or_else(|| AppError::LLMParsing("No embedding data received".into()))?
        .embedding
        .clone();

    Ok(embedding)
}

/// Generates an embedding vector using a specific model and dimension.
///
/// This is used for the re-embedding process where the model and dimensions
/// are known ahead of time and shouldn't be repeatedly fetched from settings.
pub async fn generate_embedding_with_params(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    input: &str,
    model: &str,
    dimensions: u32,
) -> Result<Vec<f32>, AppError> {
    let request = CreateEmbeddingRequestArgs::default()
        .model(model)
        .input([input])
        .dimensions(dimensions)
        .build()?;

    let response = client.embeddings().create(request).await?;

    let embedding = response
        .data
        .first()
        .ok_or_else(|| AppError::LLMParsing("No embedding data received from API".into()))?
        .embedding
        .clone();

    debug!(
        "Embedding was created with {:?} dimensions",
        embedding.len()
    );

    Ok(embedding)
}
