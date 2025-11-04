# Changelog
## Unreleased
- Added a shared `benchmarks` crate with deterministic fixtures and Criterion suites for ingestion, retrieval, and HTML handlers, plus documented baseline results for local performance checks.

## Version 0.2.6 (2025-10-29)
- Added an opt-in FastEmbed-based reranking stage behind `reranking_enabled`. It improves retrieval accuracy by re-scoring hybrid results.
- Fix: default name for relationships harmonized across application

## Version 0.2.5 (2025-10-24)
- Added manual knowledge entity creation flows using a modal, with the option for suggested relationships
- Scratchpad feature, with the feature to convert scratchpads to content.
- Added knowledge entity search results to the global search
- Backend fixes for improved performance when ingesting and retrieval

## Version 0.2.4 (2025-10-15)
- Improved retrieval performance. Ingestion and chat now utilizes full text search, vector comparison and graph traversal.
- Ingestion task archive

## Version 0.2.3 (2025-10-12)
- Fix changing vector dimensions on a fresh database (#3)

## Version 0.2.2 (2025-10-07)
- Support for ingestion of PDF files
- Improved ingestion speed
- Fix deletion of items work as expected
- Fix enabling GPT-5 use via OpenAI API

## Version 0.2.1 (2025-09-24)
- Fixed API JSON responses so iOS Shortcuts integrations keep working.

## Version 0.2.0 (2025-09-23)
- Revamped the UI with a neobrutalist theme, better dark mode, and a D3-based knowledge graph.
- Added pagination for entities and content plus new observability metrics on the dashboard.
- Enabled audio ingestion and merged the new storage backend.
- Improved performance, request filtering, and journalctl/systemd compatibility.

## Version 0.1.4 (2025-07-01)
- Added image ingestion with configurable system settings and updated Docker Compose docs.
- Hardened admin flows by fixing concurrent API/database calls and normalizing task statuses.

## Version 0.1.3 (2025-06-08)
- Added support for AI providers beyond OpenAI.
- Made the HTTP port configurable for deployments.
- Smoothed graph mapper failures, long content tiles, and refreshed project documentation.

## Version 0.1.2 (2025-05-26)
- Introduced full-text search across indexed knowledge.
- Polished the UI with consistent titles, icon fallbacks, and improved markdown scrolling.
- Fixed search result links and SurrealDB vector formatting glitches.

## Version 0.1.1 (2025-05-13)
- Added streaming feedback to ingestion tasks for clearer progress updates.
- Made the data storage path configurable.
- Improved release tooling with Chromium-enabled Nix flakes, Docker builds, and migration/template fixes.

## Version 0.1.0 (2025-05-06)
- Initial release with a SurrealDB-backed ingestion pipeline, job queue, vector search, and knowledge graph storage.
- Delivered a chat experience featuring streaming responses, conversation history, markdown rendering, and customizable system prompts.
- Introduced an admin console with analytics, registration and timezone controls, and job monitoring.
- Shipped a Tailwind/daisyUI web UI with responsive layouts, modals, content viewers, and editing flows.
- Provided readability-based content ingestion, API/HTML ingress routes, and Docker/Docker Compose tooling.
