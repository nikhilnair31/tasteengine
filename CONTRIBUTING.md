# Contributing

Small project, one maintainer. Short version: open an issue before a large PR,
keep changes reviewable, and run the two checks below before you push.

## Before you open a PR

```bash
cd backend && cargo test --release --bin taste-engine-rs   # unit tests
cd .. && make test                                          # live smoke against a running container
```

`make test` needs the service up (`make up`). It checks `/healthz`, the SPA,
that an unauthenticated API call returns 401, `/api/meta`, `/api/search`, and
the MCP tool count.

For anything that touches the container or the boot path:

```bash
make image                       # multi-stage build, ~9 min cold, cached after
docker run --rm -p 8771:8770 -v /tmp/te-try:/data taste-engine:rs
```

## Rules that keep the app honest

These are not style preferences. Each one is a bug that already happened.

- **One code path for MCP and REST.** `app/mcp.rs` calls the same functions the
  routes do. Never reimplement search, similar or retrieval in a tool.
- **Every fetch goes through the gate.** `mediaio::download_bytes` delegates to
  `gate::fetch`, which admits and reports per source. A new outbound fetch that
  bypasses it will hammer a platform after a restart.
- **No look-around in regexes.** The `regex` crate rejects them, and a bad
  pattern panics the worker that compiles it. `sources::tests::every_pattern_compiles`
  exists for that reason.
- **Originals are never re-encoded.** `save_media` writes the received bytes
  verbatim. Previews are the only derivatives. `GET /api/media/{id}/origin`
  proves it.
- **Schema changes are additive, or they need a migration.** `schema.sql` runs
  at every boot. Adding a column or a table is safe. Renaming or dropping is not,
  because existing installs have data.
- **Prompts come from `TV_PROMPTS_DIR`.** Do not hardcode a path, and keep them
  free of personal detail. They ship with the project.

## Style

- Rust 2024 edition, `cargo fmt` before you push.
- Comments explain why, not what. A comment that restates the line below it gets
  deleted.
- The SPA is vanilla JavaScript with no build step. Keep it that way.
- Errors in a route return `{"detail": "..."}` with a real status code, the same
  shape as the Python service this replaced.

## Scope

Useful PRs: a new source in `sources.rs` plus its extractor, search quality,
MCP tools, Android and extension fixes, docs.

Out of scope: screen capture and rewind (that is a separate project, LifeLog), a
desktop app, cloud sync, and anything that stores a user's platform cookies.

## Commits

Imperative subject, one concern per commit, and say why in the body. If a commit
fixes a bug found in testing, name the symptom that was visible.
