//! MCP server over streamable HTTP — port of mcp_server.py.
//!
//! Implements the JSON-RPC 2.0 surface MCP clients use: `initialize`,
//! `notifications/initialized`, `tools/list`, `tools/call`, `ping`.
//! POST /mcp with an optional Mcp-Session-Id; responses are JSON
//! (the streamable-http spec allows application/json for non-streaming
//! responses). Every tool shares the same code path as the HTTP routes.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::context;
use crate::explain;
use crate::inspire;
use crate::maintain;
use crate::mediaio;
use crate::vault;
use crate::write;

pub struct McpState {
    pub db_path: String,
    pub data_dir: std::path::PathBuf,
}

fn tool_defs() -> Value {
    json!({
        "tools": [
            {"name": "save_url", "description": "Save a post/link into the vault. Text and media get extracted, embedded (CLIP), color-profiled, and an explainer is generated in the background.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}, "title": {"type": "string", "default": ""}, "note": {"type": "string", "default": ""}, "tags": {"type": "array", "items": {"type": "string"}}, "image_urls": {"type": "array", "items": {"type": "string"}}}, "required": ["url"]}},
            {"name": "save_text", "description": "Save a text snippet or note into the vault. It joins the same vector space as the images, so it shows up in vibe searches.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "title": {"type": "string", "default": ""}, "source": {"type": "string", "default": "note"}, "tags": {"type": "array", "items": {"type": "string"}}}, "required": ["text"]}},
            {"name": "search_vault", "description": "Search the vault by theme, style, vibe, subject, or color word. Same engine as the website: keyword matches first (exact + stemmed), then CLIP visual similarity over media items.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "default": 12}, "kind": {"type": "string", "default": ""}, "source": {"type": "string", "default": ""}, "hue": {"type": "string", "default": ""}, "tag": {"type": "string", "default": ""}}, "required": ["query"]}},
            {"name": "retrieve_vault_context", "description": "Collect deduplicated vault context for a conversation. Use query for a topic or goal. Use seed_ids for selected saved items. Results include source URLs, superscript citation markers, saved text, palettes, tags, explainers, outbound links, and similarity provenance. This tool does not call a model.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string", "default": ""}, "seed_ids": {"type": "array", "items": {"type": "string"}}, "limit": {"type": "integer", "default": 12}}}},
            {"name": "get_vault_images", "description": "Return saved image previews and image blocks for selected vault items. Use this when visual inspection will improve the answer. The result includes compact metadata followed by image content when files exist.", "inputSchema": {"type": "object", "properties": {"item_ids": {"type": "array", "items": {"type": "string"}}, "limit": {"type": "integer", "default": 8}}, "required": ["item_ids"]}},
            {"name": "browse_vault", "description": "Search the vault and render ALL matches as one numbered contact-sheet image, like a Pinterest search wall — one glance instead of N image round-trips. The result carries the sheet image plus a numbered legend (n, title, kind, hue, similarity). To inspect any tile closer, call get_vault_images with that tile's item_id.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "default": 36}, "kind": {"type": "string", "default": ""}, "source": {"type": "string", "default": ""}, "hue": {"type": "string", "default": ""}, "tag": {"type": "string", "default": ""}}, "required": ["query"]}},
            {"name": "similar_items", "description": "Find visually/thematically similar items to a saved item (by id).", "inputSchema": {"type": "object", "properties": {"item_id": {"type": "string"}, "limit": {"type": "integer", "default": 10}}, "required": ["item_id"]}},
            {"name": "get_item", "description": "Full detail for one item: media, colors, tags, explainer text.", "inputSchema": {"type": "object", "properties": {"item_id": {"type": "string"}}, "required": ["item_id"]}},
            {"name": "explain_item", "description": "Queue a verbose explainer. Default returns immediately with status. Pass wait_seconds > 0 to block until done (max 300).", "inputSchema": {"type": "object", "properties": {"item_id": {"type": "string"}, "wait_seconds": {"type": "integer", "default": 0}}, "required": ["item_id"]}},
            {"name": "vault_meta", "description": "Counts by source, platform, hue, tag, kind. Good for 'what do I have'.", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "list_followups", "description": "List follow-up actions extracted from saved posts. Shopping rows include product links from Amazon, Alibaba, AliExpress, Temu, Seeed Studio, and other online stores.", "inputSchema": {"type": "object", "properties": {"status": {"type": "string", "default": "open"}, "kind": {"type": "string", "default": ""}}}},
            {"name": "delete_item", "description": "Delete an item and its media files.", "inputSchema": {"type": "object", "properties": {"item_id": {"type": "string"}}, "required": ["item_id"]}},
            {"name": "inspire", "description": "Start a design-DIRECTION job (palette, typography, one material system, optional renders). Use brief instead when the user wants something to BUILD. Default returns {job_id} immediately — poll inspire_status. Pass wait_seconds > 0 to block.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string", "default": "mixed"}, "platform": {"type": "string", "default": "any"}, "vibe": {"type": "string", "default": ""}, "style": {"type": "string", "default": ""}, "about": {"type": "string", "default": ""}, "wait_seconds": {"type": "integer", "default": 0}}}},
            {"name": "inspire_status", "description": "Check a running inspire job. Returns state + result when done.", "inputSchema": {"type": "object", "properties": {"job_id": {"type": "string"}}, "required": ["job_id"]}},
            {"name": "inspire_styles", "description": "Counts of every visual style family across the vault (gradients, typography, poster, pixel, dither, ...) with one exemplar item each.", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "brief", "description": "BUILD BRIEF from the saved taste vault. Returns ONE concrete thing to make, grounded in saved refs: kind (website/app/game/object/electronics/3dprint/interior/print/other), themes, palette hexes, type stack, parts, build steps, what to steal, and a ready build_prompt to hand to an agent. Use this for \"what should I build\", \"give me an idea\", \"fonts and palette for X\", \"gameplay idea\", \"3D print idea\". Leave about empty to get an invented suggestion. Default returns {job_id} — poll brief_status.", "inputSchema": {"type": "object", "properties": {"about": {"type": "string", "default": ""}, "seed_ids": {"type": "array", "items": {"type": "string"}}, "wait_seconds": {"type": "integer", "default": 0}}}},
            {"name": "brief_status", "description": "Check a running build-brief job. Returns state plus the brief when done. The result carries a ready build_prompt and the reference items.", "inputSchema": {"type": "object", "properties": {"job_id": {"type": "string"}}, "required": ["job_id"]}},
            {"name": "list_groups", "description": "List the vault's similarity groups (k-means clusters of the fused image+text vectors), biggest first, with a label, size, cohesion, dominant hue, and cover item ids. No model call. Use get_group or random_group to open one.", "inputSchema": {"type": "object", "properties": {"min_size": {"type": "integer", "default": 4}, "limit": {"type": "integer", "default": 60}}}},
            {"name": "random_group", "description": "Return one coherent similarity group the caller has not seen lately, with its items. No model call, no job. Use this for serendipity: 'show me something related I forgot about'. Pass exclude with group ids already shown to keep it fresh.", "inputSchema": {"type": "object", "properties": {"min_size": {"type": "integer", "default": 6}, "limit": {"type": "integer", "default": 24}, "exclude": {"type": "array", "items": {"type": "integer"}}, "kind": {"type": "string", "default": ""}, "source": {"type": "string", "default": ""}, "hue": {"type": "string", "default": ""}, "tag": {"type": "string", "default": ""}}}},
            {"name": "get_group", "description": "Open one similarity group by id: label, size, cohesion, hue, kinds, and its items as cards ordered by how close each item sits to the group center.", "inputSchema": {"type": "object", "properties": {"group_id": {"type": "integer"}, "limit": {"type": "integer", "default": 48}, "kind": {"type": "string", "default": ""}, "source": {"type": "string", "default": ""}, "hue": {"type": "string", "default": ""}, "tag": {"type": "string", "default": ""}}, "required": ["group_id"]}}
        ]
    })
}

fn tool_text(payload: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": payload.to_string()}],
        "structuredContent": payload,
    })
}

fn tool_text_plain(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}]})
}

fn tool_error(text: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": text}]})
}

fn insplit(value: &str) -> Vec<String> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect()
}

/// one tool call; conn access via the closure style used elsewhere
fn call_tool(state: &McpState, name: &str, args: &Value) -> Value {
    let conn = match Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(e) => return tool_error(&format!("db open failed: {e}")),
    };
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    let arg = |k: &str| args.get(k).cloned().unwrap_or(Value::Null);
    let astr = |k: &str| arg(k).as_str().unwrap_or("").to_string();
    match name {
        "save_url" => {
            let mut payload = json!({
                "url": astr("url"),
                "title": astr("title"),
                "note": astr("note"),
                "tags": arg("tags").as_array().cloned().unwrap_or_default(),
            });
            if let Some(imgs) = arg("image_urls").as_array() {
                if !imgs.is_empty() {
                    payload["image_urls"] = json!(imgs);
                }
            }
            match write::ensure_item(&conn, &payload) {
                Ok(v) => tool_text(&v),
                Err((code, msg)) => tool_error(&format!("save failed ({code}): {msg}")),
            }
        }
        "save_text" => {
            let text = astr("text");
            let title = {
                let t = astr("title");
                if t.is_empty() { text.chars().take(60).collect::<String>() } else { t }
            };
            let payload = json!({
                "text": text,
                "title": title,
                "source": if astr("source").is_empty() { "note".to_string() } else { astr("source") },
                "kind": "text",
                "tags": arg("tags").as_array().cloned().unwrap_or_default(),
            });
            match write::ensure_item(&conn, &payload) {
                Ok(v) => tool_text(&v),
                Err((code, msg)) => tool_error(&format!("save failed ({code}): {msg}")),
            }
        }
        "search_vault" => {
            let limit = arg("limit").as_i64().unwrap_or(12).clamp(1, 200);
            let res = vault::vault_search_pub(
                &conn,
                &astr("query"),
                "auto",
                limit,
                &astr("kind"),
                &astr("source"),
                &astr("hue"),
                &astr("tag"),
                0,
            );
            tool_text(&res)
        }
        "retrieve_vault_context" => {
            let seeds: Vec<String> = arg("seed_ids").as_array().map(|a| a.iter().filter_map(|s| s.as_str()).map(|s| s.to_string()).collect()).unwrap_or_default();
            let limit = arg("limit").as_i64().unwrap_or(12);
            tool_text(&context::vault_context(&conn, &astr("query"), &seeds, limit))
        }
        "get_vault_images" => {
            let ids: Vec<String> = arg("item_ids").as_array().map(|a| a.iter().filter_map(|s| s.as_str()).map(|s| s.to_string()).collect()).unwrap_or_default();
            let limit = arg("limit").as_i64().unwrap_or(8);
            let payload = context::vault_image_sources(&conn, &ids, limit);
            let mut out = tool_text(&payload);
            // attach image content blocks when files exist
            let mut content = out.get("content").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            for image in payload.get("images").and_then(|v| v.as_array()).into_iter().flatten() {
                let path = image.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if let Ok(bytes) = std::fs::read(path) {
                    use base64::Engine;
                    let mime = if path.to_lowercase().ends_with(".png") { "image/png" } else { "image/jpeg" };
                    content.push(json!({
                        "type": "image",
                        "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                        "mimeType": mime,
                    }));
                }
            }
            if let Some(obj) = out.as_object_mut() {
                obj.insert("content".into(), json!(content));
            }
            out
        }
        "browse_vault" => {
            let limit = arg("limit").as_i64().unwrap_or(36);
            let payload = crate::contact_sheet::build(
                &conn,
                &state.data_dir,
                &astr("query"),
                limit,
                &astr("kind"),
                &astr("source"),
                &astr("hue"),
                &astr("tag"),
            );
            let mut out = tool_text(&payload);
            if let Some(sheet) = payload.get("sheet") {
                let path = sheet.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if !path.is_empty() {
                    if let Ok(bytes) = std::fs::read(path) {
                        use base64::Engine;
                        let mut content = out.get("content").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        content.push(json!({
                            "type": "image",
                            "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                            "mimeType": "image/jpeg",
                        }));
                        if let Some(obj) = out.as_object_mut() {
                            obj.insert("content".into(), json!(content));
                        }
                    }
                }
            }
            out
        }
        "similar_items" => {
            let limit = arg("limit").as_i64().unwrap_or(10);
            match maintain::all_similar_ranked(&conn, &astr("item_id")) {
                Ok(ranked) => {
                    let page = crate::search::filter_ranked_pub(&conn, ranked, "", "", "", "", limit);
                    let items = crate::search::entries_from_ranked(&conn, page);
                    tool_text(&json!({"items": items}))
                }
                Err((code, msg)) => tool_error(&format!("similar failed ({code}): {msg}")),
            }
        }
        "get_item" => match vault::item_detail(&conn, &astr("item_id")) {
            Some(v) => tool_text(&v),
            None => tool_error("item not found"),
        },
        "explain_item" => {
            let item_id = astr("item_id");
            let exists = vault::fetch_rows(&conn, "SELECT 1 FROM items WHERE id=?", &[json!(item_id)])
                .into_iter()
                .next()
                .is_some();
            if !exists {
                return tool_text_plain("item not found");
            }
            explain::queue_explain(&conn, &item_id, true);
            let wait_seconds = arg("wait_seconds").as_i64().unwrap_or(0);
            if wait_seconds <= 0 {
                return tool_text_plain("explainer queued — poll explain_item again or open the card");
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_seconds.clamp(10, 300) as u64);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if std::time::Instant::now() >= deadline {
                    return tool_text_plain("explainer still running");
                }
                let row = vault::fetch_rows(
                    &conn,
                    "SELECT status, data FROM explanations WHERE item_id=?",
                    &[json!(item_id)],
                )
                .into_iter()
                .next();
                let Some(row) = row else { continue };
                let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if status == "done" {
                    let data: Value = serde_json::from_str(row.get("data").and_then(|v| v.as_str()).unwrap_or("{}")).unwrap_or(json!({}));
                    let mut lines: Vec<String> = Vec::new();
                    for (key, label) in [
                        ("summary", "Summary"), ("visuals", "Visuals"), ("mood", "Mood"),
                        ("style_dna", "Style DNA"), ("why_it_fits", "Why it fits you"),
                        ("color_words", "Color words"), ("video_note", "Video note"),
                    ] {
                        if let Some(val) = data.get(key) {
                            let text = match val {
                                Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "),
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            if !text.is_empty() {
                                lines.push(format!("{label}: {text}"));
                            }
                        }
                    }
                    return tool_text_plain(&lines.join("\n"));
                }
                if status == "failed" {
                    let err = serde_json::from_str::<Value>(row.get("data").and_then(|v| v.as_str()).unwrap_or("{}"))
                        .ok()
                        .and_then(|d| d.get("error").and_then(|e| e.as_str()).map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown".to_string());
                    return tool_text_plain(&format!("explainer failed: {err}"));
                }
            }
        }
        "list_groups" => {
            let min_size = arg("min_size").as_i64().unwrap_or(4);
            let limit = arg("limit").as_i64().unwrap_or(60);
            tool_text(&crate::cluster::list(&conn, min_size, limit))
        }
        "random_group" => {
            let min_size = arg("min_size").as_i64().unwrap_or(6);
            let limit = arg("limit").as_i64().unwrap_or(24);
            let exclude: Vec<i64> = arg("exclude")
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                .unwrap_or_default();
            let filter = crate::cluster::GroupFilter::from_json(args);
            match crate::cluster::random(&conn, min_size, &exclude, limit, &filter) {
                Some(v) => tool_text(&v),
                None => tool_error("no groups yet: POST /api/admin/recompute-clusters"),
            }
        }
        "get_group" => match arg("group_id").as_i64() {
            Some(id) => match crate::cluster::get(&conn, id, arg("limit").as_i64().unwrap_or(48), &crate::cluster::GroupFilter::from_json(args)) {
                Some(v) => tool_text(&v),
                None => tool_error("no such group"),
            },
            None => tool_error("group_id is required"),
        },
        "vault_meta" => tool_text(&vault::meta(&conn)),
        "list_followups" => {
            let items = vault::followups_query(&conn, &astr("status"), &astr("kind"));
            if items.is_empty() {
                let status = if astr("status").is_empty() { "open".to_string() } else { astr("status") };
                return tool_text_plain(&format!("no {status} followups"));
            }
            let mut lines: Vec<String> = Vec::new();
            for f in items {
                let label = {
                    let t = f.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    if t.is_empty() { f.get("url").and_then(|v| v.as_str()).unwrap_or("?") } else { t }
                };
                let why = f.get("why").and_then(|v| v.as_str()).unwrap_or("");
                let src = f.get("item_title").and_then(|v| v.as_str()).unwrap_or("");
                let mut row = format!("[{}] {}", f.get("kind").and_then(|v| v.as_str()).unwrap_or("?"), label);
                if let Some(url) = f.get("url").and_then(|v| v.as_str()) {
                    if !url.is_empty() {
                        row += &format!(" <{url}>");
                    }
                }
                if !src.is_empty() && src != label {
                    row += &format!(" — from: {src}");
                }
                if !why.is_empty() {
                    row += &format!(" ({why})");
                }
                lines.push(row);
            }
            tool_text_plain(&lines.join("\n"))
        }
        "delete_item" => {
            let result = write::delete_item(&conn, &astr("item_id"));
            match result {
                Ok(v) => tool_text_plain(&v.to_string()),
                Err((code, msg)) => tool_error(&format!("delete failed ({code}): {msg}")),
            }
        }
        "inspire" => {
            let params = json!({
                "project": if astr("project").is_empty() { "mixed".to_string() } else { astr("project") },
                "platform": if astr("platform").is_empty() { "any".to_string() } else { astr("platform") },
                "vibe": insplit(&astr("vibe")),
                "style": insplit(&astr("style")),
                "about": astr("about").trim().chars().take(600).collect::<String>(),
            });
            let job_id = inspire::start(&conn, &params);
            let wait_seconds = arg("wait_seconds").as_i64().unwrap_or(0);
            if wait_seconds <= 0 {
                return tool_text(&json!({"job_id": job_id, "state": "queued", "hint": "poll inspire_status(job_id)"}));
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_seconds.clamp(30, 480) as u64);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let st = inspire::status(&conn, &job_id);
                let st_state = st.get("state").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if ["done", "error", "missing"].contains(&st_state.as_str()) {
                    return tool_text(&st);
                }
                if std::time::Instant::now() >= deadline {
                    return tool_text(&json!({"job_id": job_id, "state": "working", "hint": "poll inspire_status(job_id)"}));
                }
            }
        }
        "inspire_status" => tool_text(&inspire::status(&conn, &astr("job_id"))),
        "brief" => {
            let seed_ids: Vec<String> = arg("seed_ids")
                .as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(12).collect())
                .unwrap_or_default();
            let params = json!({
                "mode": "brief",
                "project": "mixed",
                "platform": "any",
                "vibe": Vec::<String>::new(),
                "style": Vec::<String>::new(),
                "about": astr("about").trim().chars().take(600).collect::<String>(),
                "seed_id": "",
                "seed_ids": seed_ids,
            });
            let job_id = inspire::start(&conn, &params);
            let wait_seconds = arg("wait_seconds").as_i64().unwrap_or(0);
            if wait_seconds <= 0 {
                return tool_text(&json!({"job_id": job_id, "state": "queued", "hint": "poll brief_status(job_id)"}));
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_seconds.clamp(30, 480) as u64);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let st = inspire::status(&conn, &job_id);
                let st_state = st.get("state").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if ["done", "error", "missing"].contains(&st_state.as_str()) {
                    return tool_text(&st);
                }
                if std::time::Instant::now() >= deadline {
                    return tool_text(&json!({"job_id": job_id, "state": "working", "hint": "poll brief_status(job_id)"}));
                }
            }
        }
        "brief_status" => tool_text(&inspire::status(&conn, &astr("job_id"))),
        "inspire_styles" => {
            let hist = inspire::style_histogram(&conn);
            tool_text(&json!({
                "styles": hist.get("styles").cloned().unwrap_or(json!({})),
                "total": hist.get("total").cloned().unwrap_or(json!(0)),
                "projects": ["web", "app", "game", "poster", "identity", "engine", "mixed"],
                "platforms": ["web", "android", "ios", "desktop", "any"],
                "vibes": ["retro-futuristic", "brutalist", "minimal", "dark", "playful",
                          "editorial", "organic", "techy", "glassy", "noisy"],
            }))
        }
        _ => tool_error(&format!("unknown tool: {name}")),
    }
}

/// Handle one POST /mcp JSON-RPC request.
pub fn handle_post(state: &McpState, body: &[u8]) -> Response {
    let req: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return rpc_error(Value::Null, -32700, &format!("parse error: {e}"));
        }
    };
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "taste-engine", "version": "1.0.0-rs", "title": "taste-engine"},
                "instructions": "Personal taste engine: save posts, search by vibe/theme/color, retrieve compact source-linked context, inspect saved images, find similar items, and get verbose explainers.",
            });
            rpc_result(id, result)
        }
        "notifications/initialized" | "notifications/cancelled" => StatusCode::ACCEPTED.into_response(),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, tool_defs()),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(json!({}));
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let result = call_tool(state, &name, &args);
            rpc_result(id, result)
        }
        other => rpc_error(id, -32601, &format!("method not found: {other}")),
    }
}

fn rpc_result(id: Value, result: Value) -> Response {
    let body = json!({"jsonrpc": "2.0", "id": id, "result": result});
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn rpc_error(id: Value, code: i32, message: &str) -> Response {
    let body = json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}});
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// MCP auth gate: the app-level Basic auth middleware already covers /mcp.
pub fn not_found_response() -> Response {
    (StatusCode::NOT_FOUND, Body::empty()).into_response()
}
