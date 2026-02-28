use std::{path::PathBuf, sync::Arc};

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::{
    core::{
        convert::convert_asciidoc_to_markdown,
        engine::{Engine, IndexOptions},
    },
    embeddings::{self, EmbeddingConfig, EmbeddingProviderKind},
    models::AppError,
    storage::IndexStorage,
};

#[derive(Debug, Parser)]
#[command(name = "docu-retriever")]
#[command(about = "Fast local documentation retriever")]
pub struct Cli {
    #[arg(long, value_enum, default_value_t = EmbeddingBackendArg::Fastembed)]
    backend: EmbeddingBackendArg,

    #[arg(long, env = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,

    #[arg(long, default_value = "text-embedding-3-small")]
    openai_model: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EmbeddingBackendArg {
    Fastembed,
    Openai,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Convert {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Index {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = "./.doc_index")]
        db_path: PathBuf,
        #[arg(long, default_value_t = 1_200)]
        chunk_size: usize,
        #[arg(long, default_value_t = 0)]
        min_chunk_chars: usize,
        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        include_headings_in_content: bool,
    },
    Query {
        query: String,
        #[arg(long, default_value = "./.doc_index")]
        db_path: PathBuf,
        #[arg(long, default_value_t = 3)]
        top_k: usize,
        #[arg(long, default_value = "json")]
        format: String,
    },
}

pub async fn run() -> Result<(), AppError> {
    let Cli {
        backend,
        openai_api_key,
        openai_model,
        command,
    } = Cli::parse();

    match command {
        Commands::Convert { input, output } => {
            let summary = convert_asciidoc_to_markdown(&input, &output)?;
            let out = serde_json::json!({
                "status": "ok",
                "converted_files": summary.converted_files,
                "output": summary.output_dir,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&out).map_err(AppError::internal)?
            );
            Ok(())
        }
        Commands::Index {
            input,
            db_path,
            chunk_size,
            min_chunk_chars,
            include_headings_in_content,
        } => {
            let engine =
                build_engine(backend, openai_api_key.clone(), openai_model.clone()).await?;
            let summary = if chunk_size == 1_200
                && min_chunk_chars == 0
                && include_headings_in_content
            {
                engine.index(&input, &db_path).await?
            } else {
                engine
                    .index_with_options(
                        &input,
                        &db_path,
                        IndexOptions {
                            chunk_size,
                            min_chunk_chars,
                            include_headings_in_content,
                        },
                    )
                    .await?
            };
            let out = serde_json::json!({
                "status": "ok",
                "indexed_files": summary.indexed_files,
                "indexed_chunks": summary.indexed_chunks,
                "db_path": db_path,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&out).map_err(AppError::internal)?
            );
            Ok(())
        }
        Commands::Query {
            query,
            db_path,
            top_k,
            format,
        } => {
            if format.to_lowercase() != "json" {
                return Err(AppError::invalid_input(
                    "Only '--format json' is currently supported",
                ));
            }

            let engine = build_engine(backend, openai_api_key, openai_model).await?;
            let response = engine.query(&query, &db_path, top_k).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&response).map_err(AppError::internal)?
            );
            Ok(())
        }
    }
}

async fn build_engine(
    backend: EmbeddingBackendArg,
    openai_api_key: Option<String>,
    openai_model: String,
) -> Result<Engine, AppError> {
    let config = EmbeddingConfig {
        provider: match backend {
            EmbeddingBackendArg::Fastembed => EmbeddingProviderKind::FastEmbed,
            EmbeddingBackendArg::Openai => EmbeddingProviderKind::OpenAi,
        },
        openai_api_key,
        openai_model,
    };

    let provider = embeddings::build_provider(config).await?;
    let storage = Arc::new(IndexStorage::default());
    Ok(Engine::new(provider, storage))
}
