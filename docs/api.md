# REST API — Taste Engine backend (:8770)

One Rust binary serves the SPA, this API and `/mcp`.

Auth: HTTP Basic (`TV_USER:TV_PASS`) on everything except `/healthz`, the SPA
assets and `/.well-known/assetlinks.json`. `?share_token=` bypasses auth where
the route reads it.

## System

| Method | Path | Notes |
|---|---|---|
| GET | `/healthz` | open, returns 200 |
| GET | `/.well-known/assetlinks.json` | Android app links |
| GET | `/debug/embed` | CLIP text embedding for a string |

## Items

| Method | Path | Notes |
|---|---|---|
| GET | `/api/items` | grid page. `limit` defaults to 48 when absent |
| GET | `/api/items/exists?url=` | dedupe probe used by the extension |
| POST | `/api/items/upload` | multipart: file, name, mime, title, note |
| POST | `/api/items` | save a link (`url`, `title`, `note`, `tags`, `image_urls`) |
| GET | `/api/items/{id}` | detail: media, colours, tags, explainer, similar |
| DELETE | `/api/items/{id}` | soft delete (moves to trash) |
| POST | `/api/items/{id}/tags` | set tags |
| POST | `/api/items/{id}/note` | attach a note |
| POST | `/api/items/{id}/interact` | opened/liked counters |
| POST | `/api/items/{id}/explain` | regenerate explainer text only |
| POST | `/api/items/{id}/refresh` | full re-pull: media, colours, OCR, explainer, followups |
| GET | `/api/items/{id}/similar` | vector neighbours |
| POST | `/api/notes` | multipart text note; a URL in the text becomes a link save |
| GET | `/media/{name}` | media bytes (auth required) |

## Search, meta, followups

| Method | Path | Notes |
|---|---|---|
| GET | `/api/search?q=&limit=&kind=&source=&hue=&tag=` | keyword (exact + stemmed + explainer FTS), then CLIP visual |
| GET | `/api/meta` | counts by source, platform, hue, tag, kind |
| GET | `/api/followups?status=&kind=` | extracted follow-up actions |
| POST | `/api/followups/{fid}` | update status |
| POST | `/api/followups/{fid}/wiki` | resolve a follow-up to a wiki page |
| GET | `/api/settings` / PUT | per-user settings |
| GET | `/api/hue-from-hex?hex=` | colour helper |

## Trash, cleanup, duplicates

| Method | Path | Notes |
|---|---|---|
| GET | `/api/trash` | trashed items |
| POST | `/api/trash/{id}/restore` | restore one |
| POST | `/api/trash/restore-all` | restore everything |
| POST | `/api/trash/{id}/destroy` | hard delete one + its media |
| POST | `/api/trash/purge` | hard delete the whole trash |
| GET | `/api/duplicates` | near-duplicate groups |
| GET | `/api/cleanup/groups` | cleanup buckets |
| POST | `/api/cleanup/delete` | delete a selection |
| POST | `/api/cleanup/delete-type` | delete a whole bucket |

## Inspire (design direction)

| Method | Path | Notes |
|---|---|---|
| POST | `/api/inspire` | start a job, returns `job_id` |
| GET | `/api/inspire/{job_id}` | poll state + result |
| GET | `/api/inspire/meta` | style index counts |
| POST | `/api/inspire/preview` | start preview image generation |
| GET | `/api/inspire/preview/{job_id}` | poll preview |
| GET | `/api/inspire/{job_id}/export` | self-contained HTML export |

## Groups (similarity clusters)

No model call. A k-means fit over the fused image+text vectors, refreshed by
the boot fitter when the vault grows more than 10% or on demand.

| Method | Path | Notes |
|---|---|---|
| GET | `/api/groups?min_size=&limit=` | groups, biggest first: `id`, `label`, `size`, `cohesion`, `hue`, `kinds`, `covers[]` |
| GET | `/api/groups/random?min_size=&limit=&exclude=` | one group the caller has not seen lately, plus its items |
| GET | `/api/groups/{id}?limit=` | one group, items ordered by closeness to the centre |
| POST | `/api/admin/recompute-clusters` | refit now; `{ "k": 400 }` overrides the default `n/13` |
| POST | `/api/admin/recompute-fused` | rewrite item vectors with the image+text blend; `limit` defaults to 400, `refit` defaults to true |

`/api/clusters` and `/api/clusters/random` are aliases.

`item_embeds` holds `0.7 * mean(image vectors) + 0.3 * CLIP(title + body)`, so a
caption joins the same space as its picture. `POST /api/admin/recompute-fused`
is idempotent: it skips items whose title, body and media did not change.

## Ingest and admin

| Method | Path | Notes |
|---|---|---|
| POST | `/api/import` | bulk import from the extension |
| POST | `/share` | share-target endpoint (share token) |
| POST | `/api/admin/backfill-explainers` | re-queue missing explainers |
| ANY | `/mcp` | MCP streamable-HTTP endpoint |

## Behaviour rules

- `limit` defaults matter. The frontend sends none on the landing feed; a
  missing default loads one card per request.
- Route changes need a container restart. A `405` on a path you just gave a
  method to means the route is not loaded.
- Write routes trigger the pipeline: ingest → media → embed → colours → OCR →
  explainer → followups.
