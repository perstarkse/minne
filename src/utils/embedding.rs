use async_openai::types::CreateEmbeddingRequestArgs;

use crate::error::ProcessingError;

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
/// This function can return a `ProcessingError` in the following cases:
/// * If the OpenAI API request fails
/// * If the request building fails
/// * If no embedding data is received in the response
pub async fn generate_embedding(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    input: &str,
) -> Result<Vec<f32>, ProcessingError> {
    let request = CreateEmbeddingRequestArgs::default()
        .model("text-embedding-3-small")
        .input([input])
        .build()?;

    // Send the request to OpenAI
    let response = client.embeddings().create(request).await?;

    // Extract the embedding vector
    let embedding: Vec<f32> = response
        .data
        .first()
        .ok_or_else(|| ProcessingError::EmbeddingError("No embedding data received".into()))?
        .embedding
        .clone();

    Ok(embedding)
}
