# Features

## Search vs Chat

**Search** — Use when you know what you're looking for. Full-text search matches query terms across your content.

**Chat** — Use when exploring concepts or reasoning about your knowledge. The AI analyzes your query and retrieves relevant context from your entire knowledge base.

## Content Processing

Minne automatically processes saved content:

1. **Web scraping** extracts readable text from URLs (via embedded Servo engine)
2. **Text analysis** identifies key concepts and relationships
3. **Graph creation** builds connections between related content
4. **Embedding generation** enables semantic search

## Knowledge Graph

Explore your knowledge as an interactive network:

- **Manual curation** — Create entities and relationships yourself
- **AI automation** — Let AI extract entities and discover relationships
- **Hybrid approach** — AI suggests connections for your approval

The D3-based graph visualization shows entities as nodes and relationships as edges.

## Hybrid Retrieval

Minne uses hybrid retrieval over the knowledge base:

- **Vector similarity** — Semantic matching via embeddings
- **Full-text search** — Keyword matching with BM25

For **content search** (chat, global search, ingestion linking), retrieval is chunk-first: vector and FTS run over `text_chunk` rows, merged with Reciprocal Rank Fusion (RRF). When entities are needed, they are derived from the top retrieved chunks grouped by `source_id`.

For **relationship suggestions** when creating an entity, retrieval is entity-first: vector and FTS run directly over `knowledge_entity` name/description and embedding indexes, then merged with the same RRF approach.

Optional **reranking** can rescore fused chunk lists with a cross-encoder model; see below.

## Reranking (Optional)

When enabled, retrieval results are rescored with a cross-encoder model for improved relevance. Powered by [fastembed-rs](https://github.com/Anush008/fastembed-rs).

**Trade-offs:**

- Downloads ~1.1 GB of model data
- Adds latency per query
- Potentially improves answer quality, see [blog post](https://blog.stark.pub/posts/eval-retrieval-refactor/)

Enable via `RERANKING_ENABLED=true`. See [Configuration](./configuration.md).

## Multi-Format Ingestion

Supported content types:

- Plain text and notes
- URLs (web pages)
- PDF documents
- Audio files
- Images

## Scratchpad

Quickly capture content without committing to permanent storage. Convert to full content when ready.

## iOS Shortcut

Use the [Minne iOS Shortcut](https://www.icloud.com/shortcuts/e433fbd7602f4e2eaa70dca162323477) for quick content capture from your phone.
