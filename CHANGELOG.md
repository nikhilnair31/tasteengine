# Changelog

All notable changes to this project. Format follows Keep a Changelog, versions
follow semantic versioning.

## [Unreleased]

### Added

- **Groups.** k-means clusters over one fused image+text vector space, so a
  caption sits next to its picture instead of in a separate region. `GET
  /api/groups`, `/api/groups/random` (one coherent group the caller has not
  seen lately), `/api/groups/{id}`, and the `list_groups`, `random_group`,
  `get_group` MCP tools. No model call at request time: a fit takes about 3 s
  for 5300 items and runs at boot when the vault moves more than 10%.

### Changed

- Item vectors now blend the image mean (0.7) with the CLIP text embedding of
  title and body (0.3). Search and similar cards use the blended vector, so
  text on a saved post improves its neighbours. `POST
  /api/admin/recompute-fused` performs the one-time rewrite and is idempotent.
- `ORT_LIB_DIR` accepts a versioned soname (`libonnxruntime.so.1.30.0`); it
  previously required an unversioned `libonnxruntime.so`.

## [0.1.0] - 2026-10-18

First public release. Self-hosted single-user vault: one Rust binary serves the
SPA, the REST API and the MCP server on port 8770.

### Added

- **Capture.** Browser extension for Chrome, Edge, Brave, Firefox and Zen. One
  click saves a post, or pulls a whole saved collection on a schedule. Android
  app with grid, detail and share target.
- **Thirteen sources** in a declarative registry (`sources.rs`): X, Instagram,
  Pinterest, Reddit, YouTube, TikTok, Threads, Bluesky, Tumblr, Are.na, Cosmos,
  Steam, Spotify. Adding one is a row plus an extractor.
- **Search.** Keyword (exact, stemmed, explainer full-text) plus CLIP visual
  similarity in one vector space. Hue, source, kind and tag filters.
- **Color profiling.** Dominant palette, hue bucket and color words per item.
- **Explainers and follow-ups.** Per-item summary with style DNA, mood, visuals
  and tags, plus extracted actions with real store, map and stream links.
- **Inspire.** Design directions synthesized from your own references.
- **Cleanup.** Duplicate and near-duplicate detection with bulk delete.
- **20 MCP tools** over HTTP on the same port, sharing one code path with REST.
- **`te` CLI** for scripted use: `ls`, `search`, `save`, `sources`, `gates`,
  `origin`, `token`.
- **API tokens.** Revocable bearer credentials, SHA-256 at rest, plaintext shown
  once. Basic auth stays for the browser.
- **Per-source gate.** Persisted 429 cooldowns, `Retry-After` honoured, 30-minute
  challenge stop, 15-minute breaker, hourly and daily budgets, manual pause.
  A crash or restart cannot re-hammer a rate-limited platform.
- **Lossless originals.** Media is stored byte-identical. Previews are the only
  derivatives, and `GET /api/media/{id}/origin` returns a verifiable hash.

### Changed

- The container builds from source in a multi-stage image, so a clean clone
  needs no host Rust toolchain. The runtime base matches the builder's glibc.
- The SPA is baked into the image. `docker run` alone serves the UI, and Compose
  bind-mounts `web/` over it for hot editing.
- `PORT` defaults to 8770 inside the image, matching `EXPOSE` and the healthcheck.

### Removed

- Screen capture and rewind moved to a separate project (LifeLog). This repo
  never records your screen.
- The Android screenshot listener and the accessibility service went with it.

[Unreleased]: https://github.com/nikhilnair31/tasteengine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nikhilnair31/tasteengine/releases/tag/v0.1.0
