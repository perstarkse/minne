# Minne

**A graph-powered personal knowledge base that remembers for you.**

Capture content effortlessly, let AI discover connections, and explore your knowledge visually. Self-hosted and privacy-focused.

[![Release Status](https://github.com/perstarkse/minne/actions/workflows/release.yml/badge.svg)](https://github.com/perstarkse/minne/actions/workflows/release.yml)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Latest Release](https://img.shields.io/github/v/release/perstarkse/minne?sort=semver)](https://github.com/perstarkse/minne/releases/latest)

![Screenshot](./screenshot-graph.webp)

## Try It

**[Live Demo](https://minne-demo.stark.pub)** — Read-only demo deployment

## Quick Start

```bash
git clone https://github.com/perstarkse/minne.git
cd minne

# Set your OpenAI API key in docker-compose.yml, then:
docker compose up -d

# Open http://localhost:3000
```

Or with Nix (with environment variables set):

```bash
nix run 'github:perstarkse/minne#main'
```

Pre-built binaries for Windows, macOS, and Linux are available on the [Releases](https://github.com/perstarkse/minne/releases/latest) page.

## Features

- **Fast** — Rust backend with server-side rendering and HTMX for snappy interactions
- **Search & Chat** — Search or use conversational AI to find and reason about content
- **Knowledge Graph** — Visual exploration with automatic or manual relationship curation
- **Hybrid Retrieval** — Vector similarity + full-text for relevant results
- **Multi-Format** — Ingest text, URLs, PDFs, audio, and images
- **Self-Hosted** — Your data, your server, any OpenAI-compatible API

## Documentation

| Guide | Description |
|-------|-------------|
| [Installation](docs/installation.md) | Docker, Nix, binaries, source builds |
| [Configuration](docs/configuration.md) | Environment variables, config.yaml, AI setup |
| [Features](docs/features.md) | Search, Chat, Graph, Reranking, Ingestion |
| [Architecture](docs/architecture.md) | Tech stack, crate structure, data flow |
| [Vision](docs/vision.md) | Philosophy, roadmap, related projects |

## Tech Stack

Rust • Axum • HTMX • SurrealDB • FastEmbed

## Contributing

Feature requests and contributions welcome. See [Vision](docs/vision.md) for roadmap.

## License

[AGPL-3.0](LICENSE)
