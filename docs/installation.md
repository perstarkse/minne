# Installation

Minne can be installed through several methods. Choose the one that best fits your setup.

## Docker Compose (Recommended)

The fastest way to get Minne running with all dependencies:

```bash
git clone https://github.com/perstarkse/minne.git
cd minne
docker compose up -d
```

The included `docker-compose.yml` handles SurrealDB and Chromium automatically.

**Required:** Set your `OPENAI_API_KEY` in `docker-compose.yml` before starting.

## Nix

Run Minne directly with Nix (includes Chromium):

```bash
nix run 'github:perstarkse/minne#main'
```

Configure via environment variables or a `config.yaml` file. See [Configuration](./configuration.md).

## Pre-built Binaries

Download binaries for Windows, macOS, and Linux from [GitHub Releases](https://github.com/perstarkse/minne/releases/latest).

**Requirements:**
- SurrealDB instance (local or remote)
- Chromium (for web scraping)

## Build from Source

```bash
git clone https://github.com/perstarkse/minne.git
cd minne
cargo build --release --bin main
```

The binary will be at `target/release/main`.

**Requirements:**
- Rust toolchain
- SurrealDB accessible at configured address
- Chromium in PATH

## Process Modes

Minne offers flexible deployment:

| Binary | Description |
|--------|-------------|
| `main` | Combined server + worker (recommended) |
| `server` | Web interface and API only |
| `worker` | Background processing only |

For most users, `main` is the right choice. Split deployments are useful for resource optimization or scaling.

## Next Steps

- [Configuration](./configuration.md) — Environment variables and config.yaml
- [Features](./features.md) — What Minne can do
