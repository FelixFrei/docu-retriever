use async_openai::{
    config::OpenAIConfig,
    types::{CreateEmbeddingRequestArgs, EmbeddingInput},
    Client,
};
use async_trait::async_trait;

use crate::{embeddings::EmbeddingProvider, models::AppError};

pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
    model: String,
    dimension: usize,
}

impl OpenAiProvider {
    pub fn new(api_key: Option<String>, model: String) -> Result<Self, AppError> {
        let key = api_key
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                AppError::invalid_input(
                    "Missing OpenAI API key. Provide '--openai-api-key' or set OPENAI_API_KEY.",
                )
            })?;

        let client = Client::with_config(OpenAIConfig::new().with_api_key(key));

        // text-embedding-3-small default dimension
        let dimension = 1536;

        Ok(Self {
            client,
            model,
            dimension,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiProvider {
    async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let req = CreateEmbeddingRequestArgs::default()
            .model(&self.model)
            .input(EmbeddingInput::StringArray(texts))
            .build()
            .map_err(|err| {
                AppError::embedding(format!("Failed to build OpenAI embedding request: {err}"))
            })?;

        let mut res = self
            .client
            .embeddings()
            .create(req)
            .await
            .map_err(|err| AppError::embedding(format!("OpenAI embedding request failed: {err}")))?
            .data;

        res.sort_by_key(|item| item.index);
        Ok(res.into_iter().map(|item| item.embedding).collect())
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}
