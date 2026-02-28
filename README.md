# DocuRetriever (`docu-retriever`)

A local Rust CLI for indexing Markdown documentation and retrieving the most relevant sections as JSON.

The tool is designed for RAG-style workflows and AI-agent subprocess usage:
- `index`: read Markdown files, chunk text, create embeddings, store vectors in local LanceDB.
- `query`: embed a natural-language query and return top-k relevant chunks as JSON.

## Requirements

- Rust toolchain (stable recommended)
- macOS, Linux, or Windows
- `asciidoctor` and `pandoc` (required for `convert`)

This repository includes a `rust-toolchain.toml` pinned to `stable`.

## Build

```bash
cargo build --release
```

Binary path:

```bash
./target/release/docu-retriever
```

Run directly from source (without using the built binary):

```bash
cargo run -- <args>
```

## Quick Start

1. Prepare a Markdown directory (all files must be `*.md`).
2. Build the index.
3. Run queries.

If your sources are AsciiDoc, convert them first:

```bash
docu-retriever convert --input ./docs_adoc --output ./docs_md
```

### 1) Index Markdown files

```bash
docu-retriever index --input ./docs_md --db-path ./.doc_index
```

Example success output:

```json
{
  "status": "ok",
  "indexed_files": 12,
  "indexed_chunks": 248,
  "db_path": "./.doc_index"
}
```

### 2) Query the index

```bash
docu-retriever query "How is the database configured?" --db-path ./.doc_index --top-k 3 --format json
```

Example success output:

```json
{
  "query": "How is the database configured?",
  "processing_time_ms": 34,
  "results": [
    {
      "chunk_id": "cc74669e-e04a-475f-ad78-d40aa7c140db",
      "score": 0.78282654,
      "content": "Use environment variable DATABASE_URL.",
      "metadata": {
        "source_file": "setup.md",
        "original_file": "setup.adoc",
        "section": "Database Setup > Connection"
      }
    }
  ]
}
```

## Command Reference

### Global flags

- `--backend <fastembed|openai>` (default: `fastembed`)
- `--openai-api-key <KEY>` (optional; can also use `OPENAI_API_KEY`)
- `--openai-model <MODEL>` (default: `text-embedding-3-small`)

### `index`

```bash
docu-retriever index --input <DIR> [--db-path <DIR>] [--chunk-size <N>] [--min-chunk-chars <N>] [--include-headings-in-content <true|false>]
```

- `--input` required directory with `*.md` files (recursive scan)
- `--db-path` index directory (default: `./.doc_index`)
- `--chunk-size` target chunk capacity in characters (default: `1200`)
- `--min-chunk-chars` merge very small chunks into neighbors (default: `0`, disabled)
- `--include-headings-in-content <true|false>` include section headings in embedding text (default: `true`)

Chunking behavior:
- Text is first split from Markdown content and then adjacent chunks from the same file are coalesced up to `--chunk-size`.
- Coalescing can cross heading boundaries, so increasing `--chunk-size` now reliably reduces chunk count in most datasets.
- With `--include-headings-in-content=true`, each chunk is prefixed with `Section: <path>` before embedding.

How to reduce chunk count:
- Increase `--chunk-size` (bigger chunks -> fewer chunks)
- Set `--min-chunk-chars` (small chunks get merged)

Tuning examples:

```bash
# Larger chunks (usually fewer chunks overall)
docu-retriever index --input ./docs_md --db-path ./.doc_index --chunk-size 10000

# Keep headings in embedding text (default)
docu-retriever index --input ./docs_md --db-path ./.doc_index --include-headings-in-content true

# Disable heading prefix in chunk content
docu-retriever index --input ./docs_md --db-path ./.doc_index --include-headings-in-content false
```

Important:
- If you change chunking options (`--chunk-size`, `--min-chunk-chars`, `--include-headings-in-content`), re-run `index`.
- Re-indexing the same `--db-path` replaces the existing `chunks` table.

### `convert`

```bash
docu-retriever convert --input <ASCIIDOC_DIR> --output <MARKDOWN_DIR>
```

- `--input` required directory with `*.adoc` / `*.asciidoc` files (recursive scan)
- `--output` required output directory for generated `*.md` files

Example success output:

```json
{
  "status": "ok",
  "converted_files": 42,
  "output": "./docs_md"
}
```

### `query`

```bash
docu-retriever query "<question>" [--db-path <DIR>] [--top-k <N>] [--format json]
```

- positional `query` required natural-language question
- `--db-path` index directory (default: `./.doc_index`)
- `--top-k` number of results (default: `3`, must be `> 0`)
- `--format` currently only `json` is supported

## Embedding Backends

### FastEmbed (default, offline)

```bash
docu-retriever --backend fastembed index --input ./docs_md
docu-retriever --backend fastembed query "..." --db-path ./.doc_index
```

Notes:
- On first run, model artifacts are downloaded and cached locally.
- Subsequent runs are fully local/offline.

### OpenAI

```bash
export OPENAI_API_KEY="sk-..."
docu-retriever --backend openai index --input ./docs_md
docu-retriever --backend openai query "..." --db-path ./.doc_index
```

Or pass key explicitly:

```bash
docu-retriever --backend openai --openai-api-key "sk-..." query "..." --db-path ./.doc_index
```

Important:
- The same backend/model should be used for both `index` and `query`.
- Mixing dimensions causes an `EMBEDDING_ERROR`.

## Exit Codes and Error JSON

- `0`: success
- `>0`: error (JSON printed to `stdout`)

Example error:

```json
{
  "status": "error",
  "error_code": "INDEX_NOT_FOUND",
  "message": "The index at ./.doc_index could not be found. Please run the 'index' command."
}
```

Common error codes:
- `INDEX_NOT_FOUND`
- `INVALID_INPUT`
- `EMBEDDING_ERROR`
- `STORAGE_ERROR`
- `INTERNAL_ERROR`

## Notes

- Input format is Markdown only (`*.md`).
- Use `convert` first if your source docs are AsciiDoc.
- The index is stored in local LanceDB under the configured `--db-path`.
- Re-indexing the same `--db-path` overwrites the existing `chunks` table (latest run replaces previous indexed content).
- JSON output is stable and intended for machine consumers (agents/scripts).
