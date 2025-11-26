use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use async_openai::{types::CreateEmbeddingRequestArgs, Client};
use fastembed::{EmbeddingModel, ModelTrait, TextEmbedding, TextInitOptions};
use tokio::sync::Mutex;
use tracing::debug;

use crate::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingBackend {
    OpenAI,
    FastEmbed,
    Hashed,
}

impl Default for EmbeddingBackend {
    fn default() -> Self {
        Self::FastEmbed
    }
}

impl std::str::FromStr for EmbeddingBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "hashed" => Ok(Self::Hashed),
            "fastembed" | "fast-embed" | "fast" => Ok(Self::FastEmbed),
            other => Err(anyhow!(
                "unknown embedding backend '{other}'. Expected 'openai', 'hashed', or 'fastembed'."
            )),
        }
    }
}

#[derive(Clone)]
pub struct EmbeddingProvider {
    inner: EmbeddingInner,
}

#[derive(Clone)]
enum EmbeddingInner {
    OpenAI {
        client: Arc<Client<async_openai::config::OpenAIConfig>>,
        model: String,
        dimensions: u32,
    },
    Hashed {
        dimension: usize,
    },
    FastEmbed {
        model: Arc<Mutex<TextEmbedding>>,
        model_name: EmbeddingModel,
        dimension: usize,
    },
}

impl EmbeddingProvider {
    pub fn backend_label(&self) -> &'static str {
        match self.inner {
            EmbeddingInner::Hashed { .. } => "hashed",
            EmbeddingInner::FastEmbed { .. } => "fastembed",
            EmbeddingInner::OpenAI { .. } => "openai",
        }
    }

    pub fn dimension(&self) -> usize {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } => *dimension,
            EmbeddingInner::FastEmbed { dimension, .. } => *dimension,
            EmbeddingInner::OpenAI { dimensions, .. } => *dimensions as usize,
        }
    }

    pub fn model_code(&self) -> Option<String> {
        match &self.inner {
            EmbeddingInner::FastEmbed { model_name, .. } => Some(model_name.to_string()),
            EmbeddingInner::OpenAI { model, .. } => Some(model.clone()),
            EmbeddingInner::Hashed { .. } => None,
        }
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } => Ok(hashed_embedding(text, *dimension)),
            EmbeddingInner::FastEmbed { model, .. } => {
                let mut guard = model.lock().await;
                let embeddings = guard
                    .embed(vec![text.to_owned()], None)
                    .context("generating fastembed vector")?;
                embeddings
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow!("fastembed returned no embedding for input"))
            }
            EmbeddingInner::OpenAI {
                client,
                model,
                dimensions,
            } => {
                let request = CreateEmbeddingRequestArgs::default()
                    .model(model.clone())
                    .input([text])
                    .dimensions(*dimensions)
                    .build()?;

                let response = client.embeddings().create(request).await?;

                let embedding = response
                    .data
                    .first()
                    .ok_or_else(|| anyhow!("No embedding data received from OpenAI API"))?
                    .embedding
                    .clone();

                Ok(embedding)
            }
        }
    }

    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } => Ok(texts
                .into_iter()
                .map(|text| hashed_embedding(&text, *dimension))
                .collect()),
            EmbeddingInner::FastEmbed { model, .. } => {
                if texts.is_empty() {
                    return Ok(Vec::new());
                }
                let mut guard = model.lock().await;
                guard
                    .embed(texts, None)
                    .context("generating fastembed batch embeddings")
            }
            EmbeddingInner::OpenAI {
                client,
                model,
                dimensions,
            } => {
                if texts.is_empty() {
                    return Ok(Vec::new());
                }

                let request = CreateEmbeddingRequestArgs::default()
                    .model(model.clone())
                    .input(texts)
                    .dimensions(*dimensions)
                    .build()?;

                let response = client.embeddings().create(request).await?;

                let embeddings: Vec<Vec<f32>> = response
                    .data
                    .into_iter()
                    .map(|item| item.embedding)
                    .collect();

                Ok(embeddings)
            }
        }
    }

    pub async fn new_openai(
        client: Arc<Client<async_openai::config::OpenAIConfig>>,
        model: String,
        dimensions: u32,
    ) -> Result<Self> {
        Ok(EmbeddingProvider {
            inner: EmbeddingInner::OpenAI {
                client,
                model,
                dimensions,
            },
        })
    }

    pub async fn new_fastembed(model_override: Option<String>) -> Result<Self> {
        let model_name = if let Some(code) = model_override {
            EmbeddingModel::from_str(&code).map_err(|err| anyhow!(err))?
        } else {
            EmbeddingModel::default()
        };

        let options = TextInitOptions::new(model_name.clone()).with_show_download_progress(true);
        let model_name_for_task = model_name.clone();
        let model_name_code = model_name.to_string();

        let (model, dimension) = tokio::task::spawn_blocking(move || -> Result<_> {
            let model =
                TextEmbedding::try_new(options).context("initialising FastEmbed text model")?;
            let info = EmbeddingModel::get_model_info(&model_name_for_task)
                .ok_or_else(|| anyhow!("FastEmbed model metadata missing for {model_name_code}"))?;
            Ok((model, info.dim))
        })
        .await
        .context("joining FastEmbed initialisation task")??;

        Ok(EmbeddingProvider {
            inner: EmbeddingInner::FastEmbed {
                model: Arc::new(Mutex::new(model)),
                model_name,
                dimension,
            },
        })
    }

    pub fn new_hashed(dimension: usize) -> Result<Self> {
        Ok(EmbeddingProvider {
            inner: EmbeddingInner::Hashed {
                dimension: dimension.max(1),
            },
        })
    }
}

// Helper functions for hashed embeddings
fn hashed_embedding(text: &str, dimension: usize) -> Vec<f32> {
    let dim = dimension.max(1);
    let mut vector = vec![0.0f32; dim];
    if text.is_empty() {
        return vector;
    }

    let mut token_count = 0f32;
    for token in tokens(text) {
        token_count += 1.0;
        let idx = bucket(&token, dim);
        vector[idx] += 1.0;
    }

    if token_count == 0.0 {
        return vector;
    }

    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }

    vector
}

fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
}

fn bucket(token: &str, dimension: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    (hasher.finish() as usize) % dimension
}

// Backward compatibility function
pub async fn generate_embedding_with_provider(
    provider: &EmbeddingProvider,
    input: &str,
) -> Result<Vec<f32>, AppError> {
    provider.embed(input).await.map_err(AppError::from)
}

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
