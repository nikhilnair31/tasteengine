#!/usr/bin/env bash
# Back up the only stateful thing: data/vault.db + data/media.
# WAL-safe: uses sqlite's .backup, never a raw file copy.
set -euo pipefail
cd "$(dirname "$0")/.."

DEST="${1:-dist/backups/$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$DEST"

echo "backing up to $DEST"
sqlite3 data/vault.db ".backup '$DEST/vault.db'"
sqlite3 "$DEST/vault.db" 'PRAGMA integrity_check;' | head -1

if [[ -d data/media ]]; then
  rsync -a --info=progress2 data/media/ "$DEST/media/" || cp -a data/media "$DEST/"
fi

du -sh "$DEST"
echo "done"
