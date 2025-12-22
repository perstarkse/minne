# Architecture

## Tech Stack

| Layer | Technology |
|-------|------------|
| Backend | Rust with Axum (SSR) |
| Frontend | HTML + HTMX + minimal JS |
| Database | SurrealDB (graph, document, vector) |
| AI | OpenAI-compatible API |
| Web Processing | Headless Chromium |

## Crate Structure

```
minne/
├── main/                 # Combined server + worker binary
├── api-router/           # REST API routes
├── html-router/          # SSR web interface
├── ingestion-pipeline/   # Content processing pipeline
├── retrieval-pipeline/   # Search and retrieval logic
├── common/               # Shared types, storage, utilities
├── evaluations/          # Benchmarking framework
└── json-stream-parser/   # Streaming JSON utilities
```

## Process Modes

| Binary | Purpose |
|--------|---------|
| `main` | All-in-one: serves UI and processes content |
| `server` | UI and API only (no background processing) |
| `worker` | Background processing only (no UI) |

Split deployment is useful for scaling or resource isolation.

## Data Flow

```
Content In → Ingestion Pipeline → SurrealDB
                    ↓
            Entity Extraction
                    ↓
            Embedding Generation
                    ↓
            Graph Relationships

Query → Retrieval Pipeline → Results
              ↓
       Vector Search + FTS + Graph
              ↓
       RRF Fusion → (Optional Rerank) → Response
```

## Database Schema

SurrealDB stores:

- **TextContent** — Raw ingested content
- **TextChunk** — Chunked content with embeddings
- **KnowledgeEntity** — Extracted entities (people, concepts, etc.)
- **KnowledgeRelationship** — Connections between entities
- **User** — Authentication and preferences
- **SystemSettings** — Model configuration

Embeddings are stored in dedicated tables with HNSW indexes for fast vector search.

## Retrieval Strategy

1. **Collect candidates** — Vector similarity + full-text search
2. **Merge ranks** — Reciprocal Rank Fusion (RRF)
3. **Attach context** — Link chunks to parent entities
4. **Rerank** (optional) — Cross-encoder rescoring
5. **Return** — Top-k results with metadata
