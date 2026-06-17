# Evaluations

The `evaluations` crate benchmarks Minne's retrieval pipeline against standard datasets.

## Quick Start

```bash
# One-time prep (convert, slice ledger, corpus cache, DB seed)
cargo eval --warm --dataset beir --slice beir-mix-600

# Check readiness
cargo eval --status --dataset beir --slice beir-mix-600

# Run benchmark (steady state after warm)
cargo eval --dataset beir --slice beir-mix-600 --require-ready
```

Default dataset is `beir`. When `--slice` is omitted, the first catalog slice for the dataset is applied automatically (e.g. `beir-mix-600`).

Chunk-only ingestion is the default. Pass `--include-entities` to opt into entity extraction during ingestion (requires `OPENAI_API_KEY`).

### Custom slice sizes

`--slice` is a ledger id, not only a catalog name. You can use any id; `--limit` controls how many questions the ledger contains:

```bash
# 200-case BEIR mix (default --limit is 200)
cargo eval --warm --dataset beir --slice beir-mix-200
cargo eval --dataset beir --slice beir-mix-200 --require-ready
```

The catalog slice `beir-mix-600` in `manifest.yaml` is a preset with `limit: 600` and `negative_multiplier: 9.0`.

### BEIR mix layout

`beir` is a **virtual mix** across eight subset datasets (FEVER, FiQA, HotpotQA, NFCorpus, Quora, TREC-COVID, SciFact, NQ-BEIR). There is no monolithic `beir-minne/` store.

1. Build an in-memory qrels-world mix from raw subset data
2. Resolve the slice ledger (`cache/slices/beir/<slice-id>.json`)
3. Materialize only ledger paragraph ids into per-subset stores (`fever-minne/`, `fiqa-minne/`, …)
4. Ingest the slice corpus and seed SurrealDB

Conversion is **qrels-closed**: only documents that appear in qrels are exported, not the full BEIR corpus.

Chunk-only mode may evaluate fewer cases than the slice ledger size when some questions are impossible or lack verifiable answer chunks.

Reports include a **Retrieved Context Volume** section: total characters and estimated tokens across all chunks returned per query (`~chars/4`, comparable across `--chunk-result-cap` sweeps). Use this to compare the cost of raising `--chunk-result-cap`.

## Prerequisites

### SurrealDB

```bash
docker-compose up -d surrealdb
```

### Raw datasets

Place raw datasets under `evaluations/data/raw/`. See [manifest.yaml](./manifest.yaml) for paths.

BEIR subsets live in sibling directories (`data/raw/fever`, `data/raw/fiqa`, …). The `data/raw/beir` entry is a virtual catalog placeholder; warm uses the subset paths.

## Directory structure

```
evaluations/
├── data/
│   ├── raw/           # Downloaded datasets (manual)
│   │   ├── fever/     # BEIR subset raw dirs (corpus.jsonl, queries.jsonl, qrels/)
│   │   ├── fiqa/
│   │   └── …
│   └── converted/     # Sharded stores (auto-generated)
│       ├── fever-minne/  # per-BEIR-subset stores
│       ├── fiqa-minne/
│       └── …             # BEIR mix loads from subset stores (no monolithic beir-minne/)
├── cache/
│   ├── slices/        # Slice ledgers
│   └── ingested/      # Corpus ingestion caches (manifest includes namespace seed)
├── reports/           # JSON + Markdown output from benchmark runs
├── manifest.yaml
└── src/
```

**After upgrading:** delete old monolithic `*-minne.json` files, any legacy `beir-minne/` merged store, `cache/snapshots/` directories, and stale `reports/history/` artifacts, then re-run `--warm`.

## Common flags

| Flag | Description | Default |
|------|-------------|---------|
| `--dataset` | Dataset to evaluate | `beir` |
| `--slice` | Slice ledger id (catalog or custom) | first catalog slice |
| `--limit` | Max questions in the slice ledger | `200` |
| `--warm` | Prepare without running queries | — |
| `--status` | Print readiness | — |
| `--require-ready` | Fail if not warmed | — |
| `--include-entities` | Entity extraction during ingestion | off |
| `--force-convert` | Rebuild converted store | — |
| `--chunk-result-cap` | Max chunks returned per query (raise with `--k`) | `5` |
| `--perf-log-console` | Print per-stage timings after a run | off |
| `--label` | Label stored in JSON/Markdown reports | — |

See [REFACTOR.md](./REFACTOR.md) for architecture notes.
