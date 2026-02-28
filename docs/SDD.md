# Software Design Document: DocuRetriever (Rust CLI)



## 1. Introduction
### 1.1 Purpose
The `docu-retriever` CLI is a local, lightning-fast RAG (Retrieval-Augmented Generation) retrieval engine. It searches through software documentation (specifications, operations manuals) and returns the most relevant text sections as JSON to automated AI agents, providing them with precise context for their answers.

### 1.2 Core Requirements
* **Offline & Distributable:** The compiled binary and the database index must be distributable as a ready-to-use artifact (e.g., ZIP) to developer workstations (Windows/Linux/macOS).
* **Architectural Mode:** Initially implemented as an ephemeral CLI subprocess (one-off execution), but prepared at the code level for a future server extension (daemon/REST API).
* **Format Focus:** The core logic exclusively processes clean Markdown (which has been externally generated from AsciiDoc).
* **Embedding Flexibility:** By default, local, free models are used, but the architecture must allow for an easy switch to the OpenAI API.

---

## 2. Architecture & Data Flow

The system is divided into three logical phases, with Phase 1 taking place outside the Rust tool to keep the CLI highly performant and error-resistant.

### Phase 1: Preprocessing (External via CI/CD or Helper Script)
* **Problem:** AsciiDoc files contain dynamic elements (`include::[]`, variables) that are unsuitable for generating accurate embeddings.
* **Action:** An external tool (e.g., `asciidoctor` + `pandoc`) converts the AsciiDoc sources into static, flat Markdown files (`.md`), resolving all includes and variables.
* **Output:** A source directory containing pure, clean Markdown files.

### Phase 2: Indexing (`docu-retriever index`)
* **Action:** The Rust tool reads the Markdown directory.
* **Processing:** `pulldown-cmark` parses the Markdown structure (AST) to extract metadata (e.g., chapter headings). The `text-splitter` semantically chunks the text at logical boundaries (paragraphs, code blocks) without breaking context.
* **Embedding & Storage:** The chunks are vectorized and stored alongside their metadata in a local, embedded LanceDB (`.doc_index/`).

### Phase 3: Querying (`docu-retriever query`)
* **Action:** The agent calls the CLI with a natural language query.
* **Processing:** The query is vectorized using the same engine. LanceDB performs an ultra-fast vector similarity search.
* **Output:** The Top-K results are returned as structured JSON via `stdout`.

---

## 3. Technology Stack (Rust)

| Component | Crate / Technology | Justification |
| :--- | :--- | :--- |
| **CLI Framework** | `clap` | De-facto standard for building performant, well-documented CLI tools in Rust. |
| **Markdown Parsing** | `pulldown-cmark` | Generates the AST to secure heading hierarchies as metadata. |
| **Semantic Chunking** | `text-splitter` | Intelligently chunks text at Markdown boundaries, keeping code blocks intact. |
| **Embedding Engine** | `fastembed` | Runs 100% locally, highly performant, supports multilingual models (e.g., `BgeM3`). |
| **Embedding Fallback**| `async-openai` | Encapsulated via a trait, allowing usage of cloud embeddings if strictly required later. |
| **Vector Database** | `lancedb` | Serverless, stores data in a local folder, perfect for distribution (based on Apache Arrow). |
| **Output Formatting** | `serde`, `serde_json` | Ensures strict, type-safe JSON output management for the AI agent. |

---

## 4. Software Design & Code Structure

To fulfill the "CLI first, server later" requirement, the core logic is strictly separated from the CLI entry point.

### 4.1 Module Structure
* `src/main.rs`: Entry point, initializes loggers and error handling.
* `src/cli.rs`: Handles argument parsing (`clap`) and routes to `engine.rs`.
* `src/core/engine.rs`: Contains the core search and indexing logic. Designed to be called by both the CLI and a potential future web server.
* `src/embeddings/`: Contains the `EmbeddingProvider` trait and implementations for both `fastembed` and `openai`.
* `src/storage/`: Encapsulates the LanceDB logic (schema definition, insert, query).
* `src/models/`: Contains all `serde` structs for clean input and output data.

### 4.2 The Embedding Trait (Interchangeability)
To support `fastembed` as the default and `OpenAI` as a fallback, a trait is implemented:

```rust
#[async_trait::async_trait]
pub trait EmbeddingProvider {
    async fn generate_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
    fn dimension(&self) -> usize;
}
```

---

## 5. CLI Commands & Interfaces

### 5.1 Indexing
Called by the build system (CI/CD) or developer to build the database once.
```bash
docu-retriever index --input ./docs_md --db-path ./.doc_index
```

### 5.2 Querying (Subprocess Mode)
The standard call by the AI agent. Starts the tool, loads the model into RAM, searches, and exits.
```bash
docu-retriever query "What is the database setup?" --db-path ./.doc_index --top-k 3 --format json
```
*(Future extension: `docu-retriever serve --port 8080`)*

---

## 6. Data Models (JSON Interfaces)

The tool communicates with the calling agent exclusively via `stdout` using JSON. The tool exits with code `0` on success, and strictly with a code `> 0` on failure.

### 6.1 Success Output (Exit Code 0)
```json
{
  "query": "What is the database setup?",
  "processing_time_ms": 1240,
  "results": [
    {
      "chunk_id": "uuid-v4-string",
      "score": 0.89,
      "content": "The database server uses PostgreSQL. A minimum of 16GB RAM is required...",
      "metadata": {
        "source_file": "database_setup.md",
        "original_file": "database_setup.adoc",
        "section": "Database Setup > PostgreSQL Requirements"
      }
    }
  ]
}
```

### 6.2 Error Output (Exit Code > 0)
Panics and raw Rust error text on `stdout` are intercepted so the agent's JSON parser does not crash.
```json
{
  "status": "error",
  "error_code": "INDEX_NOT_FOUND",
  "message": "The index at ./.doc_index could not be found. Please run the 'index' command."
}
```

---

## 7. Distribution Strategy

The architecture allows for seamless deployment to workstations:
1. **CI/CD Build:** The pipeline converts AsciiDoc to Markdown.
2. **Index Generation:** The pipeline runs `docu-retriever index`. This generates the portable `.doc_index/` folder.
3. **Compilation:** The Rust binary `docu-retriever` is compiled for the target platforms (Windows `.exe`, Linux, macOS).
4. **Artifact Creation:** The binary and the fully computed `.doc_index/` folder are bundled into a release ZIP.
5. **Usage:** Developers and local AI agents download the ZIP, extract it, and can immediately query offline without any setup or network connection.