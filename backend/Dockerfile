# Taste Engine — Rust service image.
#
# The binary is built on the host (cargo build --release) so the image stays a
# few hundred MB instead of pulling a full Rust toolchain. Media pipeline
# helpers (ffmpeg, tesseract, yt-dlp) live in the image; models, ONNX runtime,
# data and the hot-edited SPA are bind mounts.
#
#   cargo build --release && docker compose build
FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    HOME=/home/app \
    TMPDIR=/tmp \
    LANG=C.UTF-8

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

WORKDIR /app
COPY target/release/taste-engine-rs /app/taste-engine-rs
COPY app/ /app/app/

USER 1000:1000
EXPOSE 8770

HEALTHCHECK --interval=30s --timeout=5s --start-period=25s --retries=3 \
  CMD curl -fsS http://127.0.0.1:${PORT:-8770}/healthz || exit 1

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/app/taste-engine-rs"]
