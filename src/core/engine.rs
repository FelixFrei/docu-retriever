use std::{path::Path, sync::Arc, time::Instant};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use text_splitter::{Characters, MarkdownSplitter};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    embeddings::EmbeddingProvider,
    models::{AppError, ChunkMetadata, ChunkRecord, QueryResponse},
    storage::IndexStorage,
};

#[derive(Debug, Clone)]
pub struct IndexSummary {
    pub indexed_files: usize,
    pub indexed_chunks: usize,
}

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub chunk_size: usize,
    pub min_chunk_chars: usize,
    pub include_headings_in_content: bool,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            chunk_size: 1_200,
            min_chunk_chars: 0,
            include_headings_in_content: true,
        }
    }
}

#[derive(Debug, Clone)]
struct ChunkDraft {
    content: String,
    metadata: ChunkMetadata,
}

#[derive(Debug, Clone)]
struct Section {
    section_path: String,
    content: String,
}

pub struct Engine {
    provider: Arc<dyn EmbeddingProvider>,
    storage: Arc<IndexStorage>,
}

impl Engine {
    pub fn new(provider: Arc<dyn EmbeddingProvider>, storage: Arc<IndexStorage>) -> Self {
        Self { provider, storage }
    }

    pub async fn index(&self, input: &Path, db_path: &Path) -> Result<IndexSummary, AppError> {
        self.index_with_options(input, db_path, IndexOptions::default())
            .await
    }

    pub async fn index_with_options(
        &self,
        input: &Path,
        db_path: &Path,
        options: IndexOptions,
    ) -> Result<IndexSummary, AppError> {
        if options.chunk_size == 0 {
            return Err(AppError::invalid_input(
                "chunk-size must be greater than zero",
            ));
        }
        if !input.exists() {
            return Err(AppError::invalid_input(format!(
                "Input path does not exist: {}",
                input.display()
            )));
        }

        let mut chunks = Vec::new();
        let mut indexed_files = 0;

        for entry in WalkDir::new(input)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        {
            indexed_files += 1;
            let source_file = entry
                .path()
                .strip_prefix(input)
                .unwrap_or(entry.path())
                .display()
                .to_string();
            let original_file = source_file.replacen(".md", ".adoc", 1);
            let markdown = std::fs::read_to_string(entry.path()).map_err(AppError::internal)?;

            let sections = extract_sections(&markdown);
            let splitter = MarkdownSplitter::new(Characters).with_trim_chunks(true);

            let mut file_chunks = Vec::<ChunkDraft>::new();
            for section in sections {
                let normalized = normalize_section_chunks(&splitter, &section.content, &options);

                for content in normalized {
                    let content = if options.include_headings_in_content {
                        with_section_prefix(&section.section_path, &content)
                    } else {
                        content
                    };
                    file_chunks.push(ChunkDraft {
                        content,
                        metadata: ChunkMetadata {
                            source_file: source_file.clone(),
                            original_file: original_file.clone(),
                            section: section.section_path.clone(),
                        },
                    });
                }
            }

            chunks.extend(coalesce_chunks_for_file(file_chunks, options.chunk_size));
        }

        if chunks.is_empty() {
            return Err(AppError::invalid_input(
                "No Markdown content found to index (*.md)",
            ));
        }

        let texts = chunks
            .iter()
            .map(|chunk| chunk.content.clone())
            .collect::<Vec<_>>();
        let embeddings = self.provider.generate_embeddings(texts).await?;

        if embeddings.len() != chunks.len() {
            return Err(AppError::embedding(format!(
                "Embedding count mismatch: expected {}, got {}",
                chunks.len(),
                embeddings.len()
            )));
        }

        let records = chunks
            .into_iter()
            .zip(embeddings.into_iter())
            .map(|(chunk, embedding)| ChunkRecord {
                chunk_id: Uuid::new_v4().to_string(),
                content: chunk.content,
                metadata: chunk.metadata,
                embedding,
            })
            .collect::<Vec<_>>();

        self.storage
            .persist(db_path, &records, self.provider.dimension())
            .await?;

        Ok(IndexSummary {
            indexed_files,
            indexed_chunks: records.len(),
        })
    }

    pub async fn query(
        &self,
        query: &str,
        db_path: &Path,
        top_k: usize,
    ) -> Result<QueryResponse, AppError> {
        if query.trim().is_empty() {
            return Err(AppError::invalid_input("Query must not be empty"));
        }
        if top_k == 0 {
            return Err(AppError::invalid_input("top-k must be greater than zero"));
        }

        let started_at = Instant::now();
        let index_dimension = self.storage.vector_dimension(db_path).await?;
        if index_dimension != self.provider.dimension() {
            return Err(AppError::embedding(format!(
                "Embedding dimension mismatch: index={}, provider={}",
                index_dimension,
                self.provider.dimension()
            )));
        }

        let query_embedding = self
            .provider
            .generate_embeddings(vec![query.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::embedding("Embedding backend returned no vectors"))?;

        let results = self.storage.search(db_path, query_embedding, top_k).await?;

        Ok(QueryResponse {
            query: query.to_string(),
            processing_time_ms: started_at.elapsed().as_millis(),
            results,
        })
    }
}

fn heading_level_to_index(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn extract_sections(markdown: &str) -> Vec<Section> {
    let parser = Parser::new_ext(markdown, Options::all());

    let mut sections = Vec::<Section>::new();
    let mut heading_stack = Vec::<String>::new();
    let mut current_content = String::new();
    let mut current_heading = "Document".to_string();

    let mut in_heading = false;
    let mut heading_level = HeadingLevel::H1;
    let mut heading_buf = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if !current_content.trim().is_empty() {
                    sections.push(Section {
                        section_path: current_heading.clone(),
                        content: current_content.trim().to_string(),
                    });
                    current_content.clear();
                }
                in_heading = true;
                heading_level = level;
                heading_buf.clear();
            }
            Event::End(TagEnd::Heading(..)) => {
                let lvl = heading_level_to_index(heading_level);
                let text = heading_buf.trim();
                if !text.is_empty() {
                    while heading_stack.len() >= lvl {
                        heading_stack.pop();
                    }
                    heading_stack.push(text.to_string());
                    current_heading = heading_stack.join(" > ");
                }

                in_heading = false;
                heading_buf.clear();
            }
            Event::Text(text) | Event::Code(text) => {
                if in_heading {
                    heading_buf.push_str(&text);
                } else {
                    current_content.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_heading {
                    heading_buf.push(' ');
                } else {
                    current_content.push('\n');
                }
            }
            _ => {}
        }
    }

    if !current_content.trim().is_empty() {
        sections.push(Section {
            section_path: current_heading,
            content: current_content.trim().to_string(),
        });
    }

    sections
}

fn normalize_section_chunks(
    splitter: &MarkdownSplitter<Characters>,
    section_content: &str,
    options: &IndexOptions,
) -> Vec<String> {
    if options.min_chunk_chars == 0 {
        return splitter
            .chunks(section_content, options.chunk_size)
            .filter_map(|piece| {
                let trimmed = piece.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect();
    }

    let mut out = Vec::<String>::new();
    let mut pending_small = String::new();

    for piece in splitter.chunks(section_content, options.chunk_size) {
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            continue;
        }

        let current_len = trimmed.chars().count();
        if current_len < options.min_chunk_chars {
            if !pending_small.is_empty() {
                pending_small.push('\n');
            }
            pending_small.push_str(trimmed);
            continue;
        }

        if pending_small.is_empty() {
            out.push(trimmed.to_string());
        } else {
            let merged = format!("{pending_small}\n{trimmed}");
            out.push(merged);
            pending_small.clear();
        }
    }

    if !pending_small.is_empty() {
        if let Some(last) = out.last_mut() {
            last.push('\n');
            last.push_str(&pending_small);
        } else {
            out.push(pending_small);
        }
    }

    out
}

fn coalesce_chunks_for_file(chunks: Vec<ChunkDraft>, max_chars: usize) -> Vec<ChunkDraft> {
    if chunks.len() <= 1 {
        return chunks;
    }

    let mut out = Vec::<ChunkDraft>::with_capacity(chunks.len());
    for chunk in chunks {
        if let Some(last) = out.last_mut() {
            let combined_len = last.content.chars().count() + 2 + chunk.content.chars().count();
            if combined_len <= max_chars {
                last.content.push_str("\n\n");
                last.content.push_str(&chunk.content);
                last.metadata.section =
                    merge_section_paths(&last.metadata.section, &chunk.metadata.section);
                continue;
            }
        }
        out.push(chunk);
    }

    out
}

fn with_section_prefix(section: &str, content: &str) -> String {
    format!("Section: {section}\n\n{content}")
}

fn merge_section_paths(left: &str, right: &str) -> String {
    let (start, _) = section_path_bounds(left);
    let (_, end) = section_path_bounds(right);
    if start == end {
        start.to_string()
    } else {
        format!("{start} .. {end}")
    }
}

fn section_path_bounds(section: &str) -> (&str, &str) {
    section
        .split_once(" .. ")
        .map(|(start, end)| (start, end))
        .unwrap_or((section, section))
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use async_trait::async_trait;
    use tempfile::TempDir;

    use crate::{embeddings::EmbeddingProvider, models::AppError, storage::IndexStorage};

    use super::{Engine, IndexOptions};
    use crate::models::ChunkMetadata;

    struct MockEmbeddingProvider {
        dimension: usize,
    }

    impl MockEmbeddingProvider {
        fn new(dimension: usize) -> Self {
            Self { dimension }
        }

        fn embed_text(&self, text: &str) -> Vec<f32> {
            let lower = text.to_lowercase();
            match self.dimension {
                3 => {
                    if lower.contains("database") || lower.contains("postgres") {
                        vec![1.0, 0.0, 0.0]
                    } else if lower.contains("cache") {
                        vec![0.0, 1.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 1.0]
                    }
                }
                _ => {
                    if lower.contains("database") || lower.contains("postgres") {
                        vec![1.0, 0.0, 0.0, 0.0]
                    } else if lower.contains("cache") {
                        vec![0.0, 1.0, 0.0, 0.0]
                    } else if lower.contains("auth") {
                        vec![0.0, 0.0, 1.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 0.0, 1.0]
                    }
                }
            }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, AppError> {
            Ok(texts.into_iter().map(|t| self.embed_text(&t)).collect())
        }

        fn dimension(&self) -> usize {
            self.dimension
        }
    }

    fn write_docs(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let docs_dir = tmp.path().join("docs");
        fs::create_dir_all(&docs_dir).unwrap();
        fs::write(
            docs_dir.join("setup.md"),
            "# Database Setup\n\nPostgreSQL requires 16GB RAM.\n\n## Connection\n\nSet DATABASE_URL.\n",
        )
        .unwrap();
        fs::write(
            docs_dir.join("cache.md"),
            "# Cache\n\nUse Redis for short-lived sessions.\n",
        )
        .unwrap();
        let db_path = tmp.path().join(".doc_index");
        (docs_dir, db_path)
    }

    #[tokio::test]
    async fn index_and_query_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let (docs_dir, db_path) = write_docs(&tmp);

        let engine = Engine::new(
            Arc::new(MockEmbeddingProvider::new(4)),
            Arc::new(IndexStorage::default()),
        );

        let summary = engine.index(&docs_dir, &db_path).await.unwrap();
        assert_eq!(summary.indexed_files, 2);
        assert!(summary.indexed_chunks >= 2);

        let response = engine
            .query("How is the database setup?", &db_path, 2)
            .await
            .unwrap();
        assert_eq!(response.query, "How is the database setup?");
        assert!(!response.results.is_empty());
        assert_eq!(response.results[0].metadata.source_file, "setup.md");
        assert!(response.results[0].content.starts_with("Section: "));
    }

    #[tokio::test]
    async fn index_without_headings_in_content_excludes_section_prefix() {
        let tmp = TempDir::new().unwrap();
        let (docs_dir, db_path) = write_docs(&tmp);

        let engine = Engine::new(
            Arc::new(MockEmbeddingProvider::new(4)),
            Arc::new(IndexStorage::default()),
        );

        engine
            .index_with_options(
                &docs_dir,
                &db_path,
                IndexOptions {
                    chunk_size: 1_200,
                    min_chunk_chars: 0,
                    include_headings_in_content: false,
                },
            )
            .await
            .unwrap();

        let response = engine
            .query("How is the database setup?", &db_path, 1)
            .await
            .unwrap();
        assert!(!response.results.is_empty());
        assert!(!response.results[0].content.starts_with("Section: "));
    }

    #[tokio::test]
    async fn query_missing_index_returns_index_not_found() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("missing-index");

        let engine = Engine::new(
            Arc::new(MockEmbeddingProvider::new(4)),
            Arc::new(IndexStorage::default()),
        );

        let err = engine.query("database", &missing, 1).await.unwrap_err();
        assert!(matches!(err, AppError::IndexNotFound(_)));
    }

    #[tokio::test]
    async fn query_with_dimension_mismatch_returns_embedding_error() {
        let tmp = TempDir::new().unwrap();
        let (docs_dir, db_path) = write_docs(&tmp);

        let index_engine = Engine::new(
            Arc::new(MockEmbeddingProvider::new(4)),
            Arc::new(IndexStorage::default()),
        );
        index_engine.index(&docs_dir, &db_path).await.unwrap();

        let query_engine = Engine::new(
            Arc::new(MockEmbeddingProvider::new(3)),
            Arc::new(IndexStorage::default()),
        );

        let err = query_engine
            .query("database", &db_path, 1)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Embedding(_)));
    }

    #[test]
    fn merge_section_paths_creates_compact_range() {
        assert_eq!(super::merge_section_paths("A > B", "A > B"), "A > B");
        assert_eq!(super::merge_section_paths("A > B", "A > C"), "A > B .. A > C");
        assert_eq!(
            super::merge_section_paths("A > B .. A > C", "A > D"),
            "A > B .. A > D"
        );
    }

    #[test]
    fn coalesce_chunks_for_file_merges_adjacent_sections_when_capacity_allows() {
        let chunks = vec![
            super::ChunkDraft {
                content: "A".repeat(8),
                metadata: ChunkMetadata {
                    source_file: "doc.md".to_string(),
                    original_file: "doc.adoc".to_string(),
                    section: "Intro".to_string(),
                },
            },
            super::ChunkDraft {
                content: "B".repeat(8),
                metadata: ChunkMetadata {
                    source_file: "doc.md".to_string(),
                    original_file: "doc.adoc".to_string(),
                    section: "Details".to_string(),
                },
            },
            super::ChunkDraft {
                content: "C".repeat(10),
                metadata: ChunkMetadata {
                    source_file: "doc.md".to_string(),
                    original_file: "doc.adoc".to_string(),
                    section: "Appendix".to_string(),
                },
            },
        ];

        let merged = super::coalesce_chunks_for_file(chunks, 20);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].metadata.section, "Intro .. Details");
        assert_eq!(merged[1].metadata.section, "Appendix");
    }

    #[test]
    fn with_section_prefix_adds_section_line() {
        let out = super::with_section_prefix("Guide > Setup", "Install dependencies.");
        assert_eq!(out, "Section: Guide > Setup\n\nInstall dependencies.");
    }
}
