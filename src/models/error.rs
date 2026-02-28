use anyhow::Error as AnyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    IndexNotFound(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Embedding(String),
    #[error("{0}")]
    Storage(String),
    #[error(transparent)]
    Internal(AnyError),
}

impl AppError {
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }

    pub fn index_not_found(msg: impl Into<String>) -> Self {
        Self::IndexNotFound(msg.into())
    }

    pub fn embedding(msg: impl Into<String>) -> Self {
        Self::Embedding(msg.into())
    }

    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Storage(msg.into())
    }

    pub fn internal(err: impl Into<AnyError>) -> Self {
        Self::Internal(err.into())
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::IndexNotFound(_) => "INDEX_NOT_FOUND",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Embedding(_) => "EMBEDDING_ERROR",
            Self::Storage(_) => "STORAGE_ERROR",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}
