# Changelog

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
