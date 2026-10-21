#!/usr/bin/env bash
# Download the CLIP ONNX weights into backend/models/ and verify them.
#
# The two files are ~600MB together, so they are published as release assets
# instead of living in git. Search and embedding need them; the grid, notes and
# the whole API work without them.
#
#   ./scripts/fetch-weights.sh                    # latest release of this repo
#   WEIGHTS_BASE=<url> ./scripts/fetch-weights.sh # a mirror, or a directory URL
#
# Every file is checked against backend/models/CHECKSUMS.txt. A download that
# does not match is deleted, not kept: a truncated ONNX loads as a broken model
# and the failure then shows up as nonsense search results instead of an error.
#
# To build the weights yourself instead, see backend/models/README.md.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/backend/models"
SUMS="$DEST/CHECKSUMS.txt"
REPO="${TV_REPO:-nikhilnair31/tasteengine}"
TAG="${TV_WEIGHTS_TAG:-latest}"
BASE="${WEIGHTS_BASE:-}"
MIN="${TV_WEIGHTS_MIN_BYTES:-100000000}"

FILES=(clip_text.onnx clip_vision.onnx)

if [ -z "$BASE" ]; then
  if [ "$TAG" = "latest" ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
  else
    BASE="https://github.com/$REPO/releases/download/$TAG"
  fi
fi

say() { printf '%s\n' "$*"; }
mkdir -p "$DEST"

# Expected hash for a file, from the manifest. Empty when the manifest has no
# line for it, which only happens if someone edited the file list.
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

size_of() {
  stat -c%s "$1" 2>/dev/null || stat -f%z "$1"
}

say "source: $BASE"
missing=0
for f in "${FILES[@]}"; do
  out="$DEST/$f"
  want=$(expected "$f")

  if [ -f "$out" ]; then
    size=$(size_of "$out")
    got=$(sha_of "$out")
    if [ -n "$want" ] && [ "$got" = "$want" ]; then
      say "  present  $f ($((size / 1024 / 1024))MB, checksum ok)"
      continue
    fi
    if [ -z "$want" ] && [ "$size" -ge "$MIN" ]; then
      say "  present  $f ($((size / 1024 / 1024))MB, no checksum on file)"
      continue
    fi
    say "  bad      $f (sha256 ${got:0:12}… != ${want:0:12}… ) — replacing"
    rm -f "$out"
  fi

  say "  fetching $f"
  tmp="$out.part"
  # -f so an HTML 404 page never lands in the models directory
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "$BASE/$f"; then
    rm -f "$tmp"
    say "  FAILED   $f (no bytes, or the host answered an error)"
    missing=1
    continue
  fi

  size=$(size_of "$tmp")
  if [ "$size" -lt "$MIN" ]; then
    say "  FAILED   $f is $size bytes, expected at least $MIN (an error page, or the wrong host)"
    rm -f "$tmp"
    missing=1
    continue
  fi

  if [ -n "$want" ]; then
    got=$(sha_of "$tmp")
    if [ "$got" != "$want" ]; then
      say "  FAILED   $f checksum mismatch"
      say "           expected $want"
      say "           got      $got"
      rm -f "$tmp"
      missing=1
      continue
    fi
    say "  ok       $f ($((size / 1024 / 1024))MB, checksum ok)"
  else
    say "  ok       $f ($((size / 1024 / 1024))MB, no checksum on file)"
  fi
  mv "$tmp" "$out"
done

if [ "$missing" = "1" ]; then
  say ""
  say "Some weights are missing. Either:"
  say "  1. point WEIGHTS_BASE at a URL that serves ${FILES[*]}"
  say "  2. build them from the Hugging Face model:"
  say "       pip install torch transformers sentence-transformers onnxruntime numpy"
  say "       python3 backend/scripts/export_clip.py"
  say "  3. re-run ./scripts/check-models.sh after either path"
  exit 1
fi

say ""
say "verifying"
exec "$ROOT/scripts/check-models.sh"
