mod fastembed_provider;
mod openai_provider;

use std::sync::Arc;

use async_trait::async_trait;

use crate::models::AppError;

pub use fastembed_provider::FastEmbedProvider;
pub use openai_provider::OpenAiProvider;

#[derive(Debug, Clone, Copy)]
pub enum EmbeddingProviderKind {
    FastEmbed,
    OpenAi,
}

#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProviderKind,
    pub openai_api_key: Option<String>,
    pub openai_model: String,
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, AppError>;
    fn dimension(&self) -> usize;
}

pub async fn build_provider(
    config: EmbeddingConfig,
) -> Result<Arc<dyn EmbeddingProvider>, AppError> {
    match config.provider {
        EmbeddingProviderKind::FastEmbed => Ok(Arc::new(FastEmbedProvider::new()?)),
        EmbeddingProviderKind::OpenAi => {
            let provider = OpenAiProvider::new(config.openai_api_key, config.openai_model)?;
            Ok(Arc::new(provider))
        }
    }
}
