# Installation

Minne can be installed through several methods. Choose the one that best fits your setup.

## Docker Compose (Recommended)

The fastest way to get Minne running with all dependencies:

```bash
git clone https://github.com/perstarkse/minne.git
cd minne
docker compose up -d
```

The included `docker-compose.yml` handles SurrealDB automatically.

**Required:** Set your `OPENAI_API_KEY` in `docker-compose.yml` before starting.

## Nix

Run Minne directly with Nix:

```bash
nix run 'github:perstarkse/minne#main'
```

Configure via environment variables or a `config.yaml` file. See [Configuration](./configuration.md).

## Pre-built Binaries

Download binaries for Windows, macOS (Apple Silicon), and Linux from [GitHub Releases](https://github.com/perstarkse/minne/releases/latest).

**macOS:** Release builds target `aarch64-apple-darwin` (Apple Silicon). Intel Macs can run the binary via [Rosetta 2](https://support.apple.com/en-us/102527).

**Requirements:**

- SurrealDB instance (local or remote)
- Linux: `libEGL` + `libfontconfig` for servo-fetch (bundled in release archives)
- macOS: system frameworks; ONNX Runtime is bundled in the archive `lib/` directory

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
- `libEGL` + `libfontconfig` for servo-fetch (web scraping) — bundled in Nix and Docker images

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
