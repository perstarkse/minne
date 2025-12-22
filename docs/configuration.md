# Configuration

Minne can be configured via environment variables or a `config.yaml` file. Environment variables take precedence.

## Required Settings

| Variable | Description | Example |
|----------|-------------|---------|
| `OPENAI_API_KEY` | API key for OpenAI-compatible endpoint | `sk-...` |
| `SURREALDB_ADDRESS` | WebSocket address of SurrealDB | `ws://127.0.0.1:8000` |
| `SURREALDB_USERNAME` | SurrealDB username | `root_user` |
| `SURREALDB_PASSWORD` | SurrealDB password | `root_password` |
| `SURREALDB_DATABASE` | Database name | `minne_db` |
| `SURREALDB_NAMESPACE` | Namespace | `minne_ns` |

## Optional Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `HTTP_PORT` | Server port | `3000` |
| `DATA_DIR` | Local data directory | `./data` |
| `OPENAI_BASE_URL` | Custom AI provider URL | OpenAI default |
| `RUST_LOG` | Logging level | `info` |

### Reranking (Optional)

| Variable | Description | Default |
|----------|-------------|---------|
| `RERANKING_ENABLED` | Enable FastEmbed reranking | `false` |
| `RERANKING_POOL_SIZE` | Concurrent reranker workers | `2` |
| `FASTEMBED_CACHE_DIR` | Model cache directory | `<data_dir>/fastembed/reranker` |

> [!NOTE]
> Enabling reranking downloads ~1.1 GB of model data on first startup.

## Example config.yaml

```yaml
surrealdb_address: "ws://127.0.0.1:8000"
surrealdb_username: "root_user"
surrealdb_password: "root_password"
surrealdb_database: "minne_db"
surrealdb_namespace: "minne_ns"
openai_api_key: "sk-your-key-here"
data_dir: "./minne_data"
http_port: 3000

# Optional reranking
reranking_enabled: true
reranking_pool_size: 2
```

## AI Provider Setup

Minne works with any OpenAI-compatible API that supports structured outputs.

### OpenAI (Default)

Set `OPENAI_API_KEY` only. The default base URL points to OpenAI.

### Ollama

```bash
OPENAI_API_KEY="ollama"
OPENAI_BASE_URL="http://localhost:11434/v1"
```

### Other Providers

Any provider exposing an OpenAI-compatible endpoint works. Set `OPENAI_BASE_URL` accordingly.

## Model Selection

1. Access `/admin` in your Minne instance
2. Select models for content processing and chat
3. **Content Processing**: Must support structured outputs
4. **Embedding Dimensions**: Update when changing embedding models (e.g., 1536 for `text-embedding-3-small`)
