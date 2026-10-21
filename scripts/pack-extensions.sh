#!/usr/bin/env bash
# Package both browser extensions into dist/.
#   dist/taste-engine-extension.zip  Chrome / Edge / Brave (MV3)
#   dist/taste-engine-saver.xpi      Firefox / Zen
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p dist

pack() { # src, out
  local src="$1" out="$2"
  rm -f "$out"
  (cd "$src" && zip -qr "../../$out" . -x '*.DS_Store')
  echo "  $out  ($(du -h "$out" | cut -f1))"
}

echo "packing extensions"
pack extension/chrome  dist/taste-engine-extension.zip
pack extension/firefox dist/taste-engine-saver.xpi

# Chrome loads the .zip unpacked; Firefox needs the .xpi. Keep the gecko id
# intact: never copy a chrome manifest over the firefox one.
echo "done"
