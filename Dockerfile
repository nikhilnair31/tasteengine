# Taste Engine — one image for the backend, the website and MCP.
#
# Two stages, so a clean clone builds with no host toolchain:
#   build   rust:1-bookworm    compiles the crate inside the image
#   runtime debian:bookworm-slim   ffmpeg, tesseract, yt-dlp, and both binaries
#
# The runtime base matches the builder's glibc, so the binary never depends on
# the host's libc. Data, the SPA and the CLIP weights are bind mounts.
#
#   docker compose build && docker compose up -d

# ---------------------------------------------------------------- build stage
FROM rust:1-bookworm AS build

WORKDIR /src

# Cargo files first: the dependency layer survives source edits.
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
# src/vault.rs does include_str!("../schema.sql"), so it must exist to compile.
COPY backend/schema.sql ./

RUN cargo build --release --locked --bins

# -------------------------------------------------------------- runtime stage
FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    HOME=/home/app \
    TMPDIR=/tmp \
    LANG=C.UTF-8 \
    PORT=8770

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      ffmpeg \
      libgomp1 \
      python3 \
      python3-pip \
      tesseract-ocr \
      tesseract-ocr-eng \
      tini \
 && pip3 install --no-cache-dir --break-system-packages yt-dlp \
 && rm -rf /var/lib/apt/lists/* /root/.cache

RUN useradd -m -u 1000 -d /home/app app

# ONNX Runtime, pinned and checksummed. The embedder loads it with
# `load-dynamic`, so a missing library does not stop the server: it starts, then
# panics on the first search. This used to arrive as a bind mount from a path on
# the author's own machine, which meant a fresh clone had search that crashed.
ARG ORT_VERSION=1.30.0
ARG ORT_SHA256=a5ed5a3cac51fbb2e90da632ae43d19212faaa20e76484e62bcb7c23ddb3b3fd
RUN set -eux; \
    mkdir -p /opt/ort; \
    curl -fsSL -o /tmp/ort.tgz \
      "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz"; \
    echo "${ORT_SHA256}  /tmp/ort.tgz" | sha256sum -c -; \
    tar xzf /tmp/ort.tgz --strip-components=2 -C /opt/ort \
      "onnxruntime-linux-x64-${ORT_VERSION}/lib/"; \
    rm -f /tmp/ort.tgz; \
    ls -l /opt/ort

WORKDIR /app
COPY --from=build /src/target/release/taste-engine-rs /app/taste-engine-rs
# CLI: docker exec taste-engine /app/te gates
COPY --from=build /src/target/release/te /app/te
# The SPA ships inside the image so `docker run` alone serves a UI. Compose still
# bind-mounts ./web over this for hot editing.
COPY web/ /app/frontend/
COPY backend/app/ /app/prompts/

# Paths belong in the image: without them a bare `docker run` looks for the SPA
# under $HOME/Projects/taste-engine and serves nothing. Compose sets the same
# names (it wins over these) so hot editing keeps working.
ENV TV_BASE=/app \
    TV_DATA_DIR=/data \
    TV_FRONTEND_DIR=/app/frontend \
    TV_MODELS_DIR=/app/models \
    TV_PROMPTS_DIR=/app/prompts \
    ORT_DYLIB_PATH=/opt/ort/libonnxruntime.so.1.30.0

USER 1000:1000
EXPOSE 8770

HEALTHCHECK --interval=30s --timeout=5s --start-period=25s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${PORT:-8770}/healthz" || exit 1

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/app/taste-engine-rs"]
