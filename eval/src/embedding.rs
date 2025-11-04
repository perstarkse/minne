use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use fastembed::{EmbeddingModel, ModelTrait, TextEmbedding, TextInitOptions};
use tokio::sync::Mutex;

use crate::args::{Config, EmbeddingBackend};

#[derive(Clone)]
pub struct EmbeddingProvider {
    inner: EmbeddingInner,
}

#[derive(Clone)]
enum EmbeddingInner {
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
        }
    }

    pub fn dimension(&self) -> usize {
        match &self.inner {
            EmbeddingInner::Hashed { dimension } => *dimension,
            EmbeddingInner::FastEmbed { dimension, .. } => *dimension,
        }
    }

    pub fn model_code(&self) -> Option<String> {
        match &self.inner {
            EmbeddingInner::FastEmbed { model_name, .. } => Some(model_name.to_string()),
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
        }
    }
}

pub async fn build_provider(
    config: &Config,
    default_dimension: usize,
) -> Result<EmbeddingProvider> {
    match config.embedding_backend {
        EmbeddingBackend::Hashed => Ok(EmbeddingProvider {
            inner: EmbeddingInner::Hashed {
                dimension: default_dimension.max(1),
            },
        }),
        EmbeddingBackend::FastEmbed => {
            let model_name = if let Some(code) = config.embedding_model.as_deref() {
                EmbeddingModel::from_str(code).map_err(|err| anyhow!(err))?
            } else {
                EmbeddingModel::default()
            };

            let options =
                TextInitOptions::new(model_name.clone()).with_show_download_progress(true);
            let model_name_for_task = model_name.clone();
            let model_name_code = model_name.to_string();

            let (model, dimension) = tokio::task::spawn_blocking(move || -> Result<_> {
                let model =
                    TextEmbedding::try_new(options).context("initialising FastEmbed text model")?;
                let info =
                    EmbeddingModel::get_model_info(&model_name_for_task).ok_or_else(|| {
                        anyhow!("FastEmbed model metadata missing for {model_name_code}")
                    })?;
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
    }
}

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
