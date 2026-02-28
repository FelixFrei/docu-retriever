use std::sync::Mutex;

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::{embeddings::EmbeddingProvider, models::AppError};

pub struct FastEmbedProvider {
    model: Mutex<TextEmbedding>,
    dimension: usize,
}

impl FastEmbedProvider {
    pub fn new() -> Result<Self, AppError> {
        let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15))
            .map_err(|err| {
                AppError::embedding(format!("Failed to initialize fastembed model: {err}"))
            })?;

        let probe = model.embed(vec!["dimension probe"], None).map_err(|err| {
            AppError::embedding(format!("Failed to probe fastembed model: {err}"))
        })?;

        let dimension = probe.first().map(|vec| vec.len()).ok_or_else(|| {
            AppError::embedding("Fastembed returned no vectors during initialization")
        })?;

        Ok(Self {
            model: Mutex::new(model),
            dimension,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let model = self
            .model
            .lock()
            .map_err(|_| AppError::embedding("Failed to lock fastembed model mutex"))?;

        model
            .embed(texts, None)
            .map_err(|err| AppError::embedding(format!("fastembed embedding failed: {err}")))
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}
