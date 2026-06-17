# Evaluations crate refactor plan

This document records the architecture review and the simplification work applied to the
`evaluations` crate. **No backwards compatibility** is maintained for converted JSON layouts,
legacy report history, or old cache artifact formats.

## Goals

- Smaller, linear pipeline (no state machine ceremony)
- Sharded converted store for **all** datasets (memory-efficient partial loading)
- Slice-first loading when a catalog slice is selected
- In-memory SurrealDB for ingestion (no ephemeral server namespaces)
- Single DB lifecycle module (`db/`)
- CLI helpers under `cli/`

## Primary workflow

```bash
# One-time prep (converts raw data if needed, builds slice ledger, corpus cache, DB seed)
cargo eval --warm --dataset beir --slice beir-mix-600

# Check readiness
cargo eval --status --dataset beir --slice beir-mix-600

# Steady-state benchmark
cargo eval --dataset beir --slice beir-mix-600 --require-ready
```

Default dataset is `beir`. Chunk-only ingestion is the default; pass `--include-entities` to
opt into entity extraction (requires `OPENAI_API_KEY`). Slice tuning such as
`negative_multiplier` lives in `manifest.yaml` (e.g. `beir-mix-600` uses `9.0`).

## Cache layers (after refactor)

| Layer | Location | Purpose |
|-------|----------|---------|
| Converted store | `data/converted/<name>/` | Sharded paragraphs + question catalog |
| Slice ledger | `cache/slices/<dataset>/<slice-id>.json` | Deterministic questions + paragraph set |
| Corpus cache | `cache/ingested/<dataset>/<slice-id>/` | Ingestion paragraph shards, manifest, and namespace reuse seed |

Namespace reuse state lives in the corpus manifest (`metadata.namespace_seed`), not a separate
`snapshots/` tree. After upgrading, delete old `*-minne.json` monolithic files, any
`cache/snapshots/` directories, and re-run `--warm`.

## Phases applied

### Phase 0 — dead code

- Removed unused `criterion` dependency
- Removed unused `EmbeddingCache`
- Updated README for current CLI

### Phase 1 — structure

- Flattened pipeline to linear `async fn` stages
- Removed `eval.rs` hub; imports go to owning modules
- Merged `namespace.rs`, `db_helpers.rs` → `db/`; dropped standalone `snapshot.rs`
- Moved `status.rs` → `cli/status.rs`
- Fixed catalog slice bootstrap (build ledger when explicit slice manifest is missing)

### Phase 2 — no legacy paths

- All datasets use sharded converted store only
- Removed legacy JSON layout and migration
- Removed legacy report history format
- Auto-apply first catalog slice when `--slice` omitted
- Namespace seed folded into corpus manifest (removed `cache/snapshots/`)

### Phase 3 — performance

- Ingestion always uses in-memory SurrealDB
- Slice-first partial load when ledger is complete
- Default catalog slice for dataset when `--slice` not passed
- Split `slice/` into `mod.rs`, `build.rs`, and `beir.rs`

### Phase 4 — BEIR mix slice-first

- `beir` is a virtual mix: slice ledger references prefixed ids (`fever-…`, `fiqa-…`, …)
- Conversion is **qrels-closed** per subset (only documents appearing in qrels, not full corpus)
- Slice ledger is resolved for the requested `--slice` (catalog preset or custom id + `--limit`)
- Only ledger paragraph ids are materialized into per-subset stores (`fever-minne/`, `fiqa-minne/`, …)
- No monolithic `beir-minne/` merged store
- Raw BEIR data lives in per-subset dirs under `data/raw/`; `data/raw/beir` is a catalog placeholder

## Do not re-introduce

- Monolithic `*-minne.json` converted files
- Monolithic `beir-minne/` merged converted store (use per-subset stores + virtual mix loader)
- `state-machines` pipeline for this linear flow
- `eval.rs` re-export hub
- Legacy history migration in reports
- Ephemeral `ingest_eval_*` namespaces on the shared SurrealDB server
- Separate `cache/snapshots/` namespace state files

## Open follow-ups

- Generate `DatasetKind` from `manifest.yaml` at build time
- Split `report.rs` when touching reporting again
