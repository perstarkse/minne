# === Builder Stage ===
FROM clux/muslrust:1.86.0-stable as builder 

WORKDIR /usr/src/minne
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p api-router common composite-retrieval html-router ingestion-pipeline json-stream-parser main worker
COPY api-router/Cargo.toml ./api-router/
COPY common/Cargo.toml ./common/
COPY composite-retrieval/Cargo.toml ./composite-retrieval/
COPY html-router/Cargo.toml ./html-router/
COPY ingestion-pipeline/Cargo.toml ./ingestion-pipeline/
COPY json-stream-parser/Cargo.toml ./json-stream-parser/
COPY main/Cargo.toml ./main/

# Build with the MUSL target
RUN cargo build --release --target x86_64-unknown-linux-musl --bin main --features ingestion-pipeline/docker || true

# Copy the rest of the source code
COPY . .

# Build the final application binary with the MUSL target
RUN cargo build --release --target x86_64-unknown-linux-musl --bin main --features ingestion-pipeline/docker

# === Runtime Stage ===
FROM alpine:latest

RUN apk update && apk add --no-cache \
    chromium \
    nss \
    freetype \
    harfbuzz \
    ca-certificates \
    ttf-freefont \
    font-noto-emoji \
    && \
    rm -rf /var/cache/apk/*

ENV CHROME_BIN=/usr/bin/chromium-browser \
    CHROME_PATH=/usr/lib/chromium/ \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

# Create a non-root user to run the application
RUN adduser -D -h /home/appuser appuser
WORKDIR /home/appuser
USER appuser

# Copy the compiled binary from the builder stage (note the target path)
COPY --from=builder /usr/src/minne/target/x86_64-unknown-linux-musl/release/main /usr/local/bin/main

EXPOSE 3000
# EXPOSE 8000-9000

CMD ["main"]
