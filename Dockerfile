# === Builder ===
FROM rust:1.86-bookworm AS builder
WORKDIR /usr/src/minne
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config clang cmake git && rm -rf /var/lib/apt/lists/*

# Cache deps
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p api-router common retrieval-pipeline html-router ingestion-pipeline json-stream-parser main worker
COPY api-router/Cargo.toml ./api-router/
COPY common/Cargo.toml ./common/
COPY retrieval-pipeline/Cargo.toml ./retrieval-pipeline/
COPY html-router/Cargo.toml ./html-router/
COPY ingestion-pipeline/Cargo.toml ./ingestion-pipeline/
COPY json-stream-parser/Cargo.toml ./json-stream-parser/
COPY main/Cargo.toml ./main/
RUN cargo build --release --bin main --features ingestion-pipeline/docker || true

# Build
COPY . .
RUN cargo build --release --bin main --features ingestion-pipeline/docker

# === Runtime ===
FROM debian:bookworm-slim

# Chromium + runtime deps + OpenMP for ORT
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium libnss3 libasound2 libgbm1 libxshmfence1 \
    ca-certificates fonts-dejavu fonts-noto-color-emoji \
    libgomp1 libstdc++6 curl \
  && rm -rf /var/lib/apt/lists/*

# ONNX Runtime (CPU). Change if you bump ort.
ARG ORT_VERSION=1.22.0
RUN mkdir -p /opt/onnxruntime && \
    curl -fsSL -o /tmp/ort.tgz \
      "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz" && \
    tar -xzf /tmp/ort.tgz -C /opt/onnxruntime --strip-components=1 && rm /tmp/ort.tgz

ENV CHROME_BIN=/usr/bin/chromium \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    ORT_DYLIB_PATH=/opt/onnxruntime/lib/libonnxruntime.so

# Non-root
RUN useradd -m appuser
USER appuser
WORKDIR /home/appuser

COPY --from=builder /usr/src/minne/target/release/main /usr/local/bin/main
EXPOSE 3000
CMD ["main"]
