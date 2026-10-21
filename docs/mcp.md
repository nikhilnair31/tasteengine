# MCP server — `/mcp` on :8770

Streamable-HTTP JSON-RPC. Same Basic auth as the REST API. 20 tools.
Registered in Hermes as `taste-engine` (`hermes mcp test taste-engine`).

**Rule: MCP tools and REST routes share one code path.** `search_vault` calls
`routes.vault_search()`, the exact engine behind `/api/search`. Never
re-implement search or similar inside `mcp.rs`.

## Tools

| Tool | Purpose |
|---|---|
| `save_url` | save a post/link; text and media get extracted, embedded, colour-profiled, explained |
| `save_text` | save a note into the same vector space |
| `search_vault` | theme / vibe / colour search, keyword first then CLIP |
| `browse_vault` | search rendered as ONE numbered contact sheet + legend |
| `similar_items` | vector neighbours of one item |
| `get_item` | full detail for one item |
| `get_vault_images` | image previews as MCP image blocks |
| `retrieve_vault_context` | citation-ready context bundle for a conversation |
| `explain_item` | queue an explainer (optional `wait_seconds`) |
| `vault_meta` | counts by source, platform, hue, tag, kind |
| `list_followups` | open follow-up actions with product links |
| `delete_item` | delete an item and its media |
| `brief` / `brief_status` | BUILD BRIEF: one concrete thing to make (any kind), with themes, palette, type, parts, steps, a steal list, and a ready `build_prompt`. Empty `about` = the server invents an idea |
| `list_groups` | similarity groups with a label, size, cohesion and cover ids |
| `random_group` | one coherent group the caller has not seen lately, with its items. No model call |
| `get_group` | one group by id, items ordered by closeness to the centre |
| `inspire` / `inspire_status` / `inspire_styles` | design-direction jobs (palette + typography + one material system, optional renders) and the style index |

`brief` is the build path. `inspire` is the art-direction path. Both sample the
same vault pool, but a brief also reads each reference's post text and stored
explainer, so it can answer "what job does this app do" and "how does this loop
work" — questions images alone cannot answer. Briefs never render preview images.

## Brief schema

`brief()` starts a job; `brief_status(job_id)` returns
`{ state, result: { brief, refs, brainstorm, query } }`. `brief` holds
`kind`, `title`, `one_liner`, `themes[]`, `palette[]`, `type_system`,
`build { shape, parts[] }`, `make[]`, `steal[]`, `build_prompt`, `refs[]`.
`type_system` is null when the build carries no type (most objects, prints,
electronics). Each font family carries `match: observed | closest` — `observed`
only when the saved post names the family or shows that specimen.

`GET /api/inspire/{job_id}/brief.md` returns the same brief as markdown.

## Schema trap

Tool schemas are hand-written JSON in `backend/src/mcp.rs`. Every
`properties.X` must be a dict and `required` is a ROOT-level string array. A
`required` inside `properties` makes strict clients reject the whole
`ListToolsResult`, the server parks, and the desktop Connectors page sits on
"Connecting…" forever.

After any `mcp.rs` edit: rebuild, restart the container, then
`hermes mcp test taste-engine` must show `✓ Connected` and `✓ Tools discovered: 21`.

## Transport notes

- The Rust server accepts sessionless POSTs. Python FastMCP needed
  `Accept: application/json, text/event-stream` plus a session id.
- Some Python tools returned SSE text without `structuredContent`; the Rust
  server includes it. That is a superset and harmless.
