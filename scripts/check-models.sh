#!/usr/bin/env bash
# Verify the CLIP weights exist and are the bytes this code expects.
#
#   ./scripts/check-models.sh          # sizes and sha256
#   SKIP_SHA=1 ./scripts/check-models.sh   # sizes only, for a fast pre-flight
#
# The .onnx files are gitignored (GitHub rejects blobs over 100MB), so a fresh
# clone always has to fetch or build them.
set -euo pipefail
cd "$(dirname "$0")/.."

SUMS="backend/models/CHECKSUMS.txt"
missing=0
bad=0

expected() {
  [ -f "$SUMS" ] || return 0
  # skip comments: a prose line can have the filename as its second word
  awk -v f="$1" '$1 !~ /^#/ && $2 == f { print $1 }' "$SUMS"
}

sha_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

for f in clip_text.onnx clip_vision.onnx vocab.json merges.txt; do
  p="backend/models/$f"
  if [[ ! -f "$p" ]]; then
    printf '  MISS  %s\n' "$p"
    missing=1
    continue
  fi
  size=$(du -h "$p" | cut -f1)
  if [[ -z "${SKIP_SHA:-}" ]]; then
    want=$(expected "$f")
    if [[ -z "$want" ]]; then
      printf '  ok    %-18s %s (no checksum on file)\n' "$f" "$size"
      continue
    fi
    got=$(sha_of "$p")
    if [[ "$got" != "$want" ]]; then
      printf '  BAD   %-18s %s  sha256 %s… != %s…\n' "$f" "$size" "${got:0:12}" "${want:0:12}"
      bad=1
      continue
    fi
    printf '  ok    %-18s %s  sha256 %s…\n' "$f" "$size" "${got:0:12}"
  else
    printf '  ok    %-18s %s\n' "$f" "$size"
  fi
done

if (( missing )); then
  cat <<'EOF'

CLIP weights are missing. Fetch them, or build them once:
  ./scripts/fetch-weights.sh
  # or
  pip install torch transformers sentence-transformers onnxruntime numpy
  python3 backend/scripts/export_clip.py
Without them the service still serves the grid, but search and embedding fail.
EOF
  exit 1
fi

if (( bad )); then
  cat <<'EOF'

A weight file does not match backend/models/CHECKSUMS.txt. Delete it and run
./scripts/fetch-weights.sh again. A damaged ONNX still loads, so this check is
the only thing standing between you and silently bad search results.
EOF
  exit 1
fi

echo "models ok"
