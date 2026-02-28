use serde::{Deserialize, Serialize};

use crate::models::{AppError, QueryResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub query: String,
    pub processing_time_ms: u128,
    pub results: Vec<QueryResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorOutput {
    pub status: String,
    pub error_code: String,
    pub message: String,
}

impl ErrorOutput {
    pub fn from_app_error(error: &AppError) -> Self {
        Self {
            status: "error".to_string(),
            error_code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}
