# CLIP weights

Two ONNX exports of CLIP ViT-B/32, plus the tokenizer files.

| File | Size | Used by |
|---|---|---|
| `clip_text.onnx` | 254MB | `backend/src/embed.rs` — text tower for search |
| `clip_vision.onnx` | 352MB | `backend/src/embed.rs` — vision tower, separate ORT session, opset 17 |
| `vocab.json`, `merges.txt` | 1.5MB | CLIP BPE tokenizer (tracked in git) |

The `.onnx` files are **gitignored**: GitHub rejects blobs over 100MB. A fresh
clone must copy them in before search works. `make models` checks them.

The runtime loads them through `TV_MODELS_DIR`; the decompressed sessions are
memory-mapped, so RSS is dominated by these files.
