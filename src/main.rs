mod cli;
mod core;
mod embeddings;
mod models;
mod storage;

use std::process::ExitCode;

use models::{AppError, ErrorOutput};

#[tokio::main]
async fn main() -> ExitCode {
    match cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let payload = ErrorOutput::from_app_error(&err);
            match serde_json::to_string_pretty(&payload) {
                Ok(json) => println!("{json}"),
                Err(_) => println!(
                    "{}",
                    r#"{"status":"error","error_code":"INTERNAL_ERROR","message":"Failed to serialize error output"}"#
                ),
            }

            if let AppError::Internal(inner) = err {
                eprintln!("internal error: {inner:#}");
            }
            ExitCode::from(1)
        }
    }
}
