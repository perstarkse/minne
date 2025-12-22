# Evaluations

The `evaluations` crate provides a retrieval evaluation framework for benchmarking Minne's information retrieval pipeline against standard datasets.

## Quick Start

```bash
# Run SQuAD v2.0 evaluation (vector-only, recommended)
cargo run --package evaluations -- --ingest-chunks-only

# Run a specific dataset
cargo run --package evaluations -- --dataset fiqa --ingest-chunks-only

# Convert dataset only (no evaluation)
cargo run --package evaluations -- --convert-only
```

## Prerequisites

### 1. SurrealDB

Start a SurrealDB instance before running evaluations:

```bash
docker-compose up -d surrealdb
```

Or using the default endpoint configuration:

```bash
surreal start --user root_user --pass root_password
```

### 2. Download Raw Datasets

Raw datasets must be downloaded manually and placed in `evaluations/data/raw/`. See [Dataset Sources](#dataset-sources) below for links and formats.

## Directory Structure

```
evaluations/
├── data/
│   ├── raw/          # Downloaded raw datasets (manual)
│   │   ├── squad/    # SQuAD v2.0
│   │   ├── nq-dev/   # Natural Questions
│   │   ├── fiqa/     # BEIR: FiQA-2018
│   │   ├── fever/    # BEIR: FEVER
│   │   ├── hotpotqa/ # BEIR: HotpotQA
│   │   └── ...       # Other BEIR subsets
│   └── converted/    # Auto-generated (Minne JSON format)
├── cache/            # Ingestion and embedding caches
├── reports/          # Evaluation output (JSON + Markdown)
├── manifest.yaml     # Dataset and slice definitions
└── src/              # Evaluation source code
```

## Dataset Sources

### SQuAD v2.0

Download and place at `data/raw/squad/dev-v2.0.json`:

```bash
mkdir -p evaluations/data/raw/squad
curl -L https://rajpurkar.github.io/SQuAD-explorer/dataset/dev-v2.0.json \
  -o evaluations/data/raw/squad/dev-v2.0.json
```

### Natural Questions (NQ)

Download and place at `data/raw/nq-dev/dev-all.jsonl`:

```bash
mkdir -p evaluations/data/raw/nq-dev
# Download from Google's Natural Questions page or HuggingFace
# File: dev-all.jsonl (simplified JSONL format)
```

Source: [Google Natural Questions](https://ai.google.com/research/NaturalQuestions)

### BEIR Datasets

All BEIR datasets follow the same format structure:

```
data/raw/<dataset>/
├── corpus.jsonl      # Document corpus
├── queries.jsonl     # Query set
└── qrels/
    └── test.tsv      # Relevance judgments (or dev.tsv)
```

Download datasets from the [BEIR Benchmark repository](https://github.com/beir-cellar/beir). Each dataset zip extracts to the required directory structure.

| Dataset    | Directory     |
|------------|---------------|
| FEVER      | `fever/`      |
| FiQA-2018  | `fiqa/`       |
| HotpotQA   | `hotpotqa/`   |
| NFCorpus   | `nfcorpus/`   |
| Quora      | `quora/`      |
| TREC-COVID | `trec-covid/` |
| SciFact    | `scifact/`    |
| NQ (BEIR)  | `nq/`         |

Example download:

```bash
cd evaluations/data/raw
curl -L https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/fiqa.zip -o fiqa.zip
unzip fiqa.zip && rm fiqa.zip
```

## Dataset Conversion

Raw datasets are automatically converted to Minne's internal JSON format on first run. To force reconversion:

```bash
cargo run --package evaluations -- --force-convert
```

Converted files are saved to `data/converted/` and cached for subsequent runs.

## CLI Reference

### Common Options

| Flag | Description | Default |
|------|-------------|---------|
| `--dataset <NAME>` | Dataset to evaluate | `squad-v2` |
| `--limit <N>` | Max questions to evaluate (0 = all) | `200` |
| `--k <N>` | Precision@k cutoff | `5` |
| `--slice <ID>` | Use a predefined slice from manifest | — |
| `--rerank` | Enable FastEmbed reranking stage | disabled |
| `--embedding-backend <BE>` | `fastembed` or `hashed` | `fastembed` |
| `--ingest-chunks-only` | Skip entity extraction, ingest only text chunks | disabled |

> [!TIP]
> Use `--ingest-chunks-only` when evaluating vector-only retrieval strategies. This skips the LLM-based entity extraction and graph generation, significantly speeding up ingestion while focusing on pure chunk-based vector search.

### Available Datasets

```
squad-v2, natural-questions, beir, fever, fiqa, hotpotqa, 
nfcorpus, quora, trec-covid, scifact, nq-beir
```

### Database Configuration

| Flag | Environment | Default |
|------|-------------|---------|
| `--db-endpoint` | `EVAL_DB_ENDPOINT` | `ws://127.0.0.1:8000` |
| `--db-username` | `EVAL_DB_USERNAME` | `root_user` |
| `--db-password` | `EVAL_DB_PASSWORD` | `root_password` |
| `--db-namespace` | `EVAL_DB_NAMESPACE` | auto-generated |
| `--db-database` | `EVAL_DB_DATABASE` | auto-generated |

### Example Runs

```bash
# Vector-only evaluation (recommended for benchmarking)
cargo run --package evaluations -- \
  --dataset fiqa \
  --ingest-chunks-only \
  --limit 200

# Full FiQA evaluation with reranking
cargo run --package evaluations -- \
  --dataset fiqa \
  --ingest-chunks-only \
  --limit 500 \
  --rerank \
  --k 10

# Use a predefined slice for reproducibility
cargo run --package evaluations -- --slice fiqa-test-200 --ingest-chunks-only

# Run the mixed BEIR benchmark
cargo run --package evaluations -- --dataset beir --slice beir-mix-600 --ingest-chunks-only
```

## Slices

Slices are predefined, reproducible subsets defined in `manifest.yaml`. Each slice specifies:

- **limit**: Number of questions
- **corpus_limit**: Maximum corpus size
- **seed**: Fixed RNG seed for reproducibility

View available slices in [manifest.yaml](./manifest.yaml).

## Reports

Evaluations generate reports in `reports/`:

- **JSON**: Full structured results (`*-report.json`)
- **Markdown**: Human-readable summary with sample mismatches (`*-report.md`)
- **History**: Timestamped run history (`history/`)

## Performance Tuning

```bash
# Log per-stage performance timings
cargo run --package evaluations -- --perf-log-console

# Save telemetry to file
cargo run --package evaluations -- --perf-log-json ./perf.json
```

## License

See [../LICENSE](../LICENSE).
