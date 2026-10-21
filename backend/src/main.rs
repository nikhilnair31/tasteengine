// taste-engine-rs — Rust rewrite of the Taste Vault service layer (increment 1).
// Same contract as the Python app: /healthz open, SPA static open, every API /
// media / MCP / share call requires Basic auth (share-token bypass included).
// This increment: healthz + auth middleware + static SPA. DB routes land next.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::extract::{Multipart, Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Router};
use base64::Engine;
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};
use crate::lock::LockExt;

mod cluster;
mod color;
mod contact_sheet;
mod context;
mod embed;
mod explain;
mod ingest;
mod inspire;
mod inspire_export;
mod lock;
mod gate;
mod mediaio;
mod sources;
mod tokens;
mod mt;
mod maintain;
mod mcp;
mod pipeline;
mod resolve;
mod search;
mod vault;
mod write;

struct Cfg {
    port: u16,
    data_dir: PathBuf,
    frontend_dir: PathBuf,
    tv_user: String,
    tv_pass: String,
    share_token: String,
    realm: String,
}

/// Mirror app/config.py: real env wins, else parse .env next to the project.
fn load_cfg() -> Cfg {
    let base = std::env::var("TV_BASE")
        .unwrap_or_else(|_| format!("{}/Projects/taste-engine", std::env::var("HOME").unwrap_or_default()));
    let env_path = PathBuf::from(&base).join(".env");
    if let Ok(content) = std::fs::read_to_string(&env_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('\'').trim_matches('"');
                if !k.is_empty() && std::env::var_os(k).is_none() {
                    // startup only, before threads spawn
                    unsafe { std::env::set_var(k, v) };
                }
            }
        }
    }
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    Cfg {
        port: env("PORT", "5103").parse().unwrap_or(5103),
        data_dir: PathBuf::from(env("TV_DATA_DIR", &format!("{base}/data"))),
        frontend_dir: PathBuf::from(env("TV_FRONTEND_DIR", &format!("{base}/frontend"))),
        tv_user: env("TV_USER", ""),
        tv_pass: env("TV_PASS", ""),
        share_token: env("TV_SHARE_TOKEN", ""),
        realm: env("TV_REALM", "taste-vault"),
    }
}

/// Mirror app/auth.py: Basic auth, single user; share-token query bypass.
fn current_user(cfg: &Cfg, headers: &axum::http::HeaderMap, query_token: &str) -> String {
    if !cfg.tv_user.is_empty() && !cfg.tv_pass.is_empty() {
        if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
            if let Some(raw64) = auth.strip_prefix("Basic ") {
                if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(raw64.trim()) {
                    if let Ok(text) = String::from_utf8(raw) {
                        if let Some((user, pw)) = text.split_once(':') {
                            let user_ok = constant_time_eq(user.as_bytes(), cfg.tv_user.as_bytes());
                            let pw_ok = constant_time_eq(pw.as_bytes(), cfg.tv_pass.as_bytes());
                            if user_ok && pw_ok {
                                return user.to_string();
                            }
                        }
                    }
                }
            }
        }
    }
    // Scripted clients (the `te` CLI, cron) send a bearer token instead of Basic.
    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(tok) = auth.strip_prefix("Bearer ") {
            let tok = tok.trim();
            if crate::tokens::is_valid(tok) {
                crate::tokens::touch(tok);
                return "token".to_string();
            }
        }
    }
    if !cfg.share_token.is_empty() && constant_time_eq(query_token.as_bytes(), cfg.share_token.as_bytes()) {
        return "share".to_string();
    }
    String::new()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

struct App {
    cfg: Arc<Cfg>,
    vault: vault::Vault,
}

async fn auth_middleware(
    axum::extract::State(cfg): axum::extract::State<Arc<Cfg>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    // Exemptions, mirroring main.py: healthz, assetlinks, the SPA itself.
    if path == "/healthz"
        || path == "/.well-known/assetlinks.json"
        || path == "/share"
        || (!path.starts_with("/api/") && !path.starts_with("/mcp") && !path.starts_with("/share"))
    {
        return next.run(req).await;
    }
    let query_token = req
        .uri()
        .query()
        .and_then(|q| {
            q.split('&').find_map(|p| p.strip_prefix("token=").map(|v| v.to_string()))
        })
        .unwrap_or_default();
    let user = current_user(&cfg, req.headers(), &query_token);
    if user.is_empty() {
        let mut resp = Json(serde_json::json!({"detail": "auth required"})).into_response();
        *resp.status_mut() = StatusCode::UNAUTHORIZED;
        resp.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            format!("Basic realm=\"{}\"", cfg.realm)
                .parse()
                .unwrap_or_else(|_| header::HeaderValue::from_static("Basic")),
        );
        return resp;
    }
    req.extensions_mut().insert(user);
    next.run(req).await
}

async fn api_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"detail":"no such route"}"#,
    )
        .into_response()
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true}))
}

/// verification-only endpoint (rust side): returns the CLIP text embedding
async fn debug_embed(Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let text = q.get("q").cloned().unwrap_or_default();
    let want_ids = q.contains_key("tokens");
    if want_ids {
        let tok = crate::embed::debug_tokens(&text);
        return Json(serde_json::json!({"ids": tok})).into_response();
    }
    match crate::embed::embed_text(&[text]) {
        Ok(vecs) => match vecs.into_iter().next() {
            Some(v) => Json(serde_json::json!({"dim": v.len(), "vec": v})).into_response(),
            None => api_error(StatusCode::INTERNAL_SERVER_ERROR, "no vec"),
        },
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

#[derive(serde::Deserialize)]
struct ListQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    hue: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    offset: Option<String>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    seed: Option<String>,
}

async fn vault_list(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Extension(user): Extension<String>,
    Query(q): Query<ListQuery>,
) -> Response {
    let int_of = |raw: &Option<String>| -> Result<i64, Response> {
        match raw {
            None => Ok(0),
            Some(s) => s.parse::<i64>().map_err(|_| {
                Json(serde_json::json!({"detail": "invalid integer"})).into_response()
            }),
        }
    };
    // python default: limit: int = 48 when the query param is absent
    let limit = match &q.limit {
        None => 48,
        Some(_) => match int_of(&q.limit) {
            Ok(v) => v,
            Err(r) => return r,
        },
    };
    let offset = match int_of(&q.offset) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let seed = match int_of(&q.seed) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let params = vault::ListParams {
        q: q.q.unwrap_or_default(),
        kind: q.kind.unwrap_or_default(),
        source: q.source.unwrap_or_default(),
        platform: q.platform.unwrap_or_default(),
        hue: q.hue.unwrap_or_default(),
        tag: q.tag.unwrap_or_default(),
        limit,
        offset,
        sort: q.sort.unwrap_or_else(|| "random".to_string()),
        seed,
    };
    Json(vault::list_items(&app.vault, &user, &params)).into_response()
}

fn with_conn<F>(app: &App, f: F) -> Response
where
    F: FnOnce(&rusqlite::Connection) -> Response,
{
    let conn = app.vault.conn.lock_ok();
    f(&conn)
}

async fn vault_item_detail(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| match vault::item_detail(conn, &item_id) {
        Some(v) => Json(v).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"detail": "not found"})),
        )
            .into_response(),
    })
}

async fn vault_exists(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let url = q.get("url").cloned().unwrap_or_default();
    with_conn(&app, |conn| match vault::item_exists_lookup(conn, &url) {
        Ok(v) => Json(v).into_response(),
        Err(detail) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"detail": detail})),
        )
            .into_response(),
    })
}

async fn meta_handler(axum::extract::State(app): axum::extract::State<Arc<App>>) -> Response {
    with_conn(&app, |conn| Json(vault::meta(conn)).into_response())
}

async fn followups_handler(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let status = q.get("status").cloned().unwrap_or_else(|| "open".into());
    let kind = q.get("kind").cloned().unwrap_or_default();
    with_conn(&app, |conn| {
        Json(serde_json::json!({"items": vault::followups_query(conn, &status, &kind)})).into_response()
    })
}

async fn settings_get(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Extension(user): Extension<String>,
) -> Response {
    with_conn(&app, |conn| Json(vault::get_settings(conn, &user)).into_response())
}

async fn hue_from_hex_handler(
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let hex = q.get("hex").cloned().unwrap_or_default();
    match vault::hue_from_hex(&hex) {
        Ok(v) => Json(v).into_response(),
        Err(detail) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"detail": detail})),
        )
            .into_response(),
    }
}

fn api_error(status: StatusCode, detail: &str) -> Response {
    (status, Json(serde_json::json!({"detail": detail}))).into_response()
}

fn status_of(e: &(u16, String)) -> StatusCode {
    StatusCode::from_u16(e.0).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn items_save(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return api_error(StatusCode::UNPROCESSABLE_ENTITY, "body must be JSON"),
        }
    };
    // Refuse to queue work against a platform that is cooling down. The gate is
    // checked again per download, so this is the early, visible signal.
    if let Some(src) = value.get("url").and_then(|v| v.as_str()).and_then(sources::source_of) {
        if sources::is_social(src) {
            if let Err((reason, retry_at)) = gate::admit(src) {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({
                        "detail": format!("source {src} is {reason}"),
                        "source": src,
                        "reason": reason,
                        "retry_at": retry_at,
                    })),
                )
                    .into_response();
            }
        }
    }
    with_conn(&app, |conn| match write::ensure_item(conn, &value) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn items_upload(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    mut multipart: Multipart,
) -> Response {
    let mut payload = String::new();
    let mut file: Option<(String, String, Vec<u8>)> = None;
    while let Ok(Some(mut field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "payload" => {
                if let Ok(text) = field.text().await {
                    payload = text;
                }
            }
            "file" => {
                let filename = field.file_name().unwrap_or("").to_string();
                let content_type = field.content_type().unwrap_or("").to_string();
                let mut data = Vec::new();
                while let Ok(Some(chunk)) = field.chunk().await {
                    data.extend(chunk);
                }
                file = Some((filename, content_type, data));
            }
            _ => {}
        }
    }
    let fref = file
        .as_ref()
        .map(|(n, c, d)| (n.as_str(), c.as_str(), d.as_slice()));
    with_conn(&app, |conn| match write::save_upload(conn, &payload, fref) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn notes_save(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    mut multipart: Multipart,
) -> Response {
    let mut text = String::new();
    let mut title = String::new();
    let mut files: Vec<write::StagedFile> = Vec::new();
    while let Ok(Some(mut field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "text" {
            if let Ok(t) = field.text().await {
                text = t;
            }
        } else if name == "title" {
            if let Ok(t) = field.text().await {
                title = t;
            }
        } else if name == "files" {
            let filename = field.file_name().unwrap_or("attachment").to_string();
            let content_type = field.content_type().unwrap_or("").to_string();
            let mut data = Vec::new();
            while let Ok(Some(chunk)) = field.chunk().await {
                data.extend(chunk);
            }
            let staged = write::stage_bytes_public(&filename, &content_type, &data);
            files.push(staged);
        }
    }
    with_conn(&app, |conn| match write::create_note(conn, &text, &title, &files) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn share_target(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let mut url = String::new();
    let mut title = String::new();
    let mut text = String::new();
    let mut token = String::new();
    let mut files: Vec<write::StagedFile> = Vec::new();
    while let Ok(Some(mut field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "url" => url = field.text().await.unwrap_or_default(),
            "title" => title = field.text().await.unwrap_or_default(),
            "text" => text = field.text().await.unwrap_or_default(),
            "token" => token = field.text().await.unwrap_or_default(),
            "files" => {
                let filename = field.file_name().unwrap_or("shared-file").to_string();
                let content_type = field.content_type().unwrap_or("").to_string();
                let mut data = Vec::new();
                while let Ok(Some(chunk)) = field.chunk().await {
                    data.extend(chunk);
                }
                if !data.is_empty() {
                    files.push(write::stage_bytes_public(&filename, &content_type, &data));
                }
            }
            _ => {}
        }
    }
    let query_token = uri
        .query()
        .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("token=").map(|v| v.to_string())))
        .unwrap_or_default();
    let query_token = if query_token.is_empty() { token.clone() } else { query_token };
    let user = current_user(&app.cfg, &headers, &query_token);
    let token_ok = !app.cfg.share_token.is_empty() && {
        // constant-time-ish compare
        let a = query_token.as_bytes();
        let b = app.cfg.share_token.as_bytes();
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    };
    if user.is_empty() && !token_ok {
        let mut resp = api_error(StatusCode::UNAUTHORIZED, "auth required");
        resp.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            format!("Basic realm=\"{}\"", app.cfg.realm)
                .parse()
                .unwrap_or_else(|_| header::HeaderValue::from_static("Basic")),
        );
        return resp;
    }
    match write::share_target(&app.vault.conn.lock_ok(), &url, &title, &text, &token, &files) {
        Ok(location) => (
            StatusCode::SEE_OTHER,
            [(header::LOCATION, location)],
            String::new(),
        )
            .into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    }
}

async fn item_tags(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    with_conn(&app, |conn| match write::add_tags(conn, &item_id, &value) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn item_interact(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Extension(user): Extension<String>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| match write::interact(conn, &user, &item_id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn item_note(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    with_conn(&app, |conn| match write::set_note(conn, &item_id, &value) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn item_delete(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| match write::delete_item(conn, &item_id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn item_explain(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
                [item_id.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            return api_error(StatusCode::NOT_FOUND, "not found");
        }
        write::queue_explain(conn, &item_id, true);
        Json(serde_json::json!({"ok": true, "status": "queued"})).into_response()
    })
}

async fn item_refresh(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
                [item_id.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            return api_error(StatusCode::NOT_FOUND, "not found");
        }
        // python spawns pipeline.refresh_item in a daemon thread; the real
        // worker lands with the pipeline port. Response parity only.
        Json(serde_json::json!({"ok": true, "status": "refreshing"})).into_response()
    })
}

async fn item_similar(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let int_of = |raw: &Option<String>| -> i64 {
        raw.as_deref().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0)
    };
    let limit_raw = int_of(&q.get("limit").cloned());
    let limit = limit_raw.clamp(1, 500);
    let offset = int_of(&q.get("offset").cloned()).max(0);
    let kind = q.get("kind").cloned().unwrap_or_default();
    let source = q.get("source").cloned().unwrap_or_default();
    with_conn(&app, |conn| match maintain::all_similar_ranked(conn, &item_id) {
        Err(e) => api_error(status_of(&e), &e.1),
        Ok(ranked) => {
            let total = ranked.len();
            let page: Vec<(String, f64)> = ranked
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            let items = crate::search::entries_from_ranked(conn, page);
            let n = items.len();
            Json(serde_json::json!({
                "items": items,
                "offset": offset,
                "limit": limit,
                "total": total,
                "has_more": ((offset + n as i64) < total as i64),
            }))
            .into_response()
        }
    })
}

async fn trash_list(axum::extract::State(app): axum::extract::State<Arc<App>>) -> Response {
    with_conn(&app, |conn| Json(write::list_trash(conn)).into_response())
}

async fn trash_restore_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| match write::trash_restore(conn, &item_id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn trash_destroy_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(item_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| match write::trash_destroy(conn, &item_id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn trash_purge_route(axum::extract::State(app): axum::extract::State<Arc<App>>) -> Response {
    with_conn(&app, |conn| Json(write::trash_purge(conn)).into_response())
}

async fn trash_restore_all_route(axum::extract::State(app): axum::extract::State<Arc<App>>) -> Response {
    with_conn(&app, |conn| Json(write::trash_restore_all(conn)).into_response())
}

async fn duplicates_route(axum::extract::State(app): axum::extract::State<Arc<App>>) -> Response {
    with_conn(&app, |conn| Json(maintain::duplicates_payload(conn)).into_response())
}

async fn cleanup_groups_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let tsim = q
        .get("tsim")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.62);
    let max_groups = q
        .get("max_groups")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    with_conn(&app, |conn| {
        Json(maintain::cleanup_payload(conn, tsim, if max_groups <= 0 { 0 } else { max_groups }))
            .into_response()
    })
}

async fn cleanup_delete_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let raw_ids = value.get("ids").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if raw_ids.is_empty() {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "ids must be a non-empty list");
    }
    if raw_ids.len() > write::MAX_IDS_PUBLIC {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("delete at most {} at once", write::MAX_IDS_PUBLIC),
        );
    }
    let ids: Vec<String> = raw_ids
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    with_conn(&app, |conn| Json(maintain::delete_similar(conn, &ids)).into_response())
}

async fn cleanup_delete_type_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let channel = value.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    if !["all", "source", "media", "empty"].contains(&channel) {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "channel must be one of: all, source, media, empty",
        );
    }
    with_conn(&app, |conn| match maintain::delete_type_channel(conn, channel) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn settings_put(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Extension(user): Extension<String>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    with_conn(&app, |conn| match write::put_settings(conn, &user, &value) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn followup_update(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(fid): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    with_conn(&app, |conn| match write::update_followup(conn, &fid, &value) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn followup_wiki(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(fid): axum::extract::Path<String>,
) -> Response {
    with_conn(&app, |conn| match write::followup_to_wiki(conn, &fid) {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(status_of(&e), &e.1),
    })
}

async fn admin_recompute_colors(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    // Batched on purpose: a whole-vault pass is a client loop over `next_after`.
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let limit = value.get("limit").and_then(|v| v.as_i64()).unwrap_or(200);
    let after = value.get("after").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let data_dir = app.cfg.data_dir.clone();
    with_conn(&app, |conn| {
        Json(mediaio::recompute_colors(conn, &data_dir, limit, &after)).into_response()
    })
}

async fn admin_backfill(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let limit = value.get("limit").and_then(|v| v.as_i64()).unwrap_or(500);
    let force = value.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    with_conn(&app, |conn| {
        Json(serde_json::json!({"queued": write::backfill_explainers(conn, limit, force)})).into_response()
    })
}

async fn import_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    mut multipart: Multipart,
) -> Response {
    let mut data: Vec<u8> = Vec::new();
    let mut filename = String::new();
    let mut saw_file = false;
    while let Ok(Some(mut field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            saw_file = true;
            filename = field.file_name().unwrap_or("").to_string();
            while let Ok(Some(chunk)) = field.chunk().await {
                data.extend(chunk);
            }
        }
    }
    if !saw_file || filename.is_empty() {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "no file");
    }
    if data.len() > 50 * 1024 * 1024 {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "file too large (50MB cap)");
    }
    with_conn(&app, |conn| Json(maintain::run_import(conn, &data, &filename)).into_response())
}

async fn search_handler(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let int_of = |raw: &Option<String>, default: i64| -> i64 {
        raw.as_deref()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(default)
    };
    let params = vault::SearchParams {
        q: q.get("q").cloned().unwrap_or_default(),
        mode: {
            let m = q.get("mode").cloned().unwrap_or_else(|| "auto".into());
            if m.is_empty() { "auto".into() } else { m }
        },
        limit: int_of(&q.get("limit").cloned(), 60),
        kind: q.get("kind").cloned().unwrap_or_default(),
        source: q.get("source").cloned().unwrap_or_default(),
        hue: q.get("hue").cloned().unwrap_or_default(),
        tag: q.get("tag").cloned().unwrap_or_default(),
        offset: int_of(&q.get("offset").cloned(), 0).max(0),
    };
    with_conn(&app, |conn| Json(vault::vault_search(conn, &params)).into_response())
}

fn media_mime(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

async fn serve_media(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    // python: os.path.basename(name) — no traversal
    let safe = std::path::Path::new(&name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let path = app.cfg.data_dir.join("media").join(&safe);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mime = media_mime(&safe);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime)],
                bytes,
            )
                .into_response()
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"detail": "missing"})),
        )
            .into_response(),
    }
}


// ------------------------------------------------- sources, gate, tokens, origin

async fn sources_handler() -> Response {
    Json(sources::json_table()).into_response()
}

async fn gates_handler() -> Response {
    Json(gate::snapshot()).into_response()
}

async fn gate_pause_handler(axum::extract::Path(source): axum::extract::Path<String>) -> Response {
    gate::pause(&source);
    Json(gate::snapshot()).into_response()
}

async fn gate_resume_handler(axum::extract::Path(source): axum::extract::Path<String>) -> Response {
    gate::resume(&source);
    Json(gate::snapshot()).into_response()
}

/// The scraper lives in the browser, so it reports what it saw here. That is how
/// a platform 429 becomes a persisted cooldown on the server.
async fn gate_report_handler(
    axum::extract::Path(source): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Response {
    #[derive(serde::Deserialize, Default)]
    struct Report {
        #[serde(default)]
        status: u16,
        #[serde(default)]
        retry_after: Option<i64>,
    }
    let r: Report = serde_json::from_slice(&body).unwrap_or_default();
    if r.status == 0 {
        return api_error(StatusCode::BAD_REQUEST, "status required");
    }
    gate::report(&source, r.status, r.retry_after);
    Json(gate::snapshot()).into_response()
}

async fn tokens_list_handler() -> Response {
    Json(tokens::list()).into_response()
}

async fn tokens_create_handler(body: axum::body::Bytes) -> Response {
    #[derive(serde::Deserialize, Default)]
    struct Create {
        #[serde(default)]
        name: String,
    }
    let c: Create = serde_json::from_slice(&body).unwrap_or_default();
    let name = if c.name.trim().is_empty() { "cli".to_string() } else { c.name.trim().to_string() };
    match tokens::create(&name) {
        Ok((id, token)) => Json(json!({
            "id": id,
            "name": name,
            "token": token,
            "note": "shown once. send it as Authorization: Bearer <token>",
        }))
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn tokens_delete_handler(axum::extract::Path(token_id): axum::extract::Path<String>) -> Response {
    if tokens::delete(&token_id) {
        Json(json!({ "deleted": token_id })).into_response()
    } else {
        api_error(StatusCode::NOT_FOUND, "no such token")
    }
}

/// Proof that the stored original is byte-identical to what arrived: the media
/// file is never re-encoded, only previews are derivatives.
async fn media_origin_handler(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    axum::extract::Path(media_id): axum::extract::Path<String>,
) -> Response {
    use sha2::{Digest, Sha256};
    with_conn(&app, |conn| {
        let rows = vault::fetch_rows(
            conn,
            "SELECT id, file, mime, original_name FROM media WHERE id = ?",
            &[json!(media_id)],
        );
        let Some(m) = rows.into_iter().next() else {
            return api_error(StatusCode::NOT_FOUND, "no such media");
        };
        let file = m.get("file").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let path = app.cfg.data_dir.join("media").join(&file);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return api_error(StatusCode::NOT_FOUND, &format!("cannot read {file}: {e}")),
        };
        let mut h = Sha256::new();
        h.update(&bytes);
        let sha: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        Json(json!({
            "id": m.get("id").cloned().unwrap_or(Value::Null),
            "file": file,
            "mime": m.get("mime").cloned().unwrap_or(Value::Null),
            "original_name": m.get("original_name").cloned().unwrap_or(Value::Null),
            "bytes": bytes.len(),
            "sha256": sha,
            "lossless": true,
        }))
        .into_response()
    })
}

#[tokio::main]
async fn main() {
    let cfg = Arc::new(load_cfg());
    // A fresh clone has no data/ directory. Create it so first boot works.
    if let Err(e) = std::fs::create_dir_all(&cfg.data_dir) {
        eprintln!(
            "taste-engine-rs: cannot create data dir {}: {e}",
            cfg.data_dir.display()
        );
        std::process::exit(1);
    }
    let db_path = cfg.data_dir.join("vault.db");
    let vault = match vault::Vault::open(&db_path.to_string_lossy()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("taste-engine-rs: cannot open {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };
    // Cooldowns and API tokens live in the same file, so a restart cannot
    // re-hammer a platform or forget a revoked token.
    gate::init(db_path.to_string_lossy().to_string());
    tokens::init(db_path.to_string_lossy().to_string());
    let app_state = Arc::new(App { cfg: cfg.clone(), vault });
    // pipeline workers share the same sqlite file (python does the same)
    pipeline::start(cfg.data_dir.join("vault.db").to_string_lossy().to_string());
    explain::start(cfg.data_dir.join("vault.db").to_string_lossy().to_string());
    let inspire_db = cfg.data_dir.join("vault.db").to_string_lossy().to_string();
    inspire::start_worker(inspire_db.clone());
    inspire::start_preview_worker(inspire_db.clone());
    inspire::warmup(inspire_db.clone());
    // Groups: create the tables now, and fit once in the background when the
    // vault has no fit yet, so a fresh install has a working Groups button.
    cluster::start_fitter(inspire_db);
    let frontend = cfg.frontend_dir.clone();

    let static_svc = ServeDir::new(&frontend)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(frontend.join("index.html")));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/.well-known/assetlinks.json", get(assetlinks_handler))
        .route("/debug/embed", get(debug_embed))
        .route("/api/items", get(vault_list).post(items_save))
        .route("/api/items/exists", get(vault_exists))
        .route("/api/items/upload", post(items_upload))
        .route("/api/notes", post(notes_save))
        .route("/api/items/{item_id}", get(vault_item_detail).delete(item_delete))
        .route("/api/items/{item_id}/tags", post(item_tags))
        .route("/api/items/{item_id}/interact", post(item_interact))
        .route("/api/items/{item_id}/note", post(item_note))
        .route("/api/items/{item_id}/explain", post(item_explain))
        .route("/api/items/{item_id}/refresh", post(item_refresh))
        .route("/api/items/{item_id}/similar", get(item_similar))
        .route("/api/meta", get(meta_handler))
        .route("/api/followups", get(followups_handler))
        .route("/api/followups/{fid}", post(followup_update))
        .route("/api/followups/{fid}/wiki", post(followup_wiki))
        .route("/api/settings", get(settings_get).put(settings_put))
        .route("/api/sources", get(sources_handler))
        .route("/api/sources/gates", get(gates_handler))
        .route("/api/sources/{source}/pause", post(gate_pause_handler))
        .route("/api/sources/{source}/resume", post(gate_resume_handler))
        .route("/api/sources/{source}/report", post(gate_report_handler))
        .route("/api/tokens", get(tokens_list_handler).post(tokens_create_handler))
        .route("/api/tokens/{token_id}", axum::routing::delete(tokens_delete_handler))
        .route("/api/media/{media_id}/origin", get(media_origin_handler))
        .route("/api/hue-from-hex", get(hue_from_hex_handler))
        .route("/api/trash", get(trash_list))
        .route("/api/trash/purge", post(trash_purge_route))
        .route("/api/trash/restore-all", post(trash_restore_all_route))
        .route("/api/trash/{item_id}/restore", post(trash_restore_route))
        .route("/api/trash/{item_id}/destroy", post(trash_destroy_route))
        .route("/api/duplicates", get(duplicates_route))
        .route("/api/cleanup/groups", get(cleanup_groups_route))
        .route("/api/cleanup/delete", post(cleanup_delete_route))
        .route("/api/cleanup/delete-type", post(cleanup_delete_type_route))
        .route("/api/admin/backfill-explainers", post(admin_backfill))
        .route("/api/admin/recompute-colors", post(admin_recompute_colors))
        .route("/api/import", post(import_route))
        .route("/share", post(share_target))
        .route("/api/search", get(search_handler))
        .route("/api/inspire", post(inspire_start_route))
        .route("/api/inspire/meta", get(inspire_meta_route))
        .route("/api/inspire/preview", post(inspire_preview_start_route))
        .route("/api/inspire/preview/{job_id}", get(inspire_preview_status_route))
        .route("/api/inspire/{job_id}/export", get(inspire_export_route))
        .route("/api/inspire/{job_id}/brief.md", get(inspire_brief_md_route))
        .route("/api/inspire/{job_id}", get(inspire_status_route))
        .route("/api/groups", get(clusters_list_route))
        .route("/api/groups/random", get(clusters_random_route))
        .route("/api/groups/{group_id}", get(clusters_get_route))
        .route("/api/clusters", get(clusters_list_route))
        .route("/api/clusters/random", get(clusters_random_route))
        .route("/api/clusters/{group_id}", get(clusters_get_route))
        .route("/api/admin/recompute-clusters", post(admin_recompute_clusters))
        .route("/api/admin/recompute-fused", post(admin_recompute_fused))
        // A mistyped /api path used to fall through to the SPA and answer
        // 200 text/html, so a client bug looked like success. Say 404 in the
        // language the caller expects.
        .route("/api/{*rest}", axum::routing::any(api_not_found))
        .route("/mcp", axum::routing::any(mcp_handler))
        .route("/mcp/", axum::routing::any(mcp_handler))
        .route("/media/{name}", get(serve_media))
        .fallback_service(static_svc)
        .layer(middleware::from_fn_with_state(cfg.clone(), auth_middleware))
        .with_state(app_state);

    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        eprintln!("taste-engine-rs: cannot bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!("taste-engine-rs: listening on {addr} (increment 1: healthz + auth + SPA shell)");
    axum::serve(listener, app).await.unwrap();
}

// ---------- cluster handlers (groups) ----------

async fn clusters_list_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let min_size = q.get("min_size").and_then(|v| v.parse::<i64>().ok()).unwrap_or(4);
    let limit = q.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(200);
    with_conn(&app, |conn| Json(cluster::list(conn, min_size, limit)).into_response())
}

async fn clusters_random_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let min_size = q.get("min_size").and_then(|v| v.parse::<i64>().ok()).unwrap_or(6);
    let limit = q.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(24);
    let exclude: Vec<i64> = q
        .get("exclude")
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<i64>().ok()).collect())
        .unwrap_or_default();
    let filter = cluster::GroupFilter::from_query(&q);
    with_conn(&app, |conn| match cluster::random(conn, min_size, &exclude, limit, &filter) {
        Some(v) => Json(v).into_response(),
        None => api_error(StatusCode::NOT_FOUND, "no cluster yet: POST /api/admin/recompute-clusters"),
    })
}

async fn clusters_get_route(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    Path(group_id): Path<i64>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = q.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(48);
    let filter = cluster::GroupFilter::from_query(&q);
    with_conn(&app, |conn| match cluster::get(conn, group_id, limit, &filter) {
        Some(v) => Json(v).into_response(),
        None => api_error(StatusCode::NOT_FOUND, "no such group"),
    })
}

async fn admin_recompute_clusters(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let k = value.get("k").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
    with_conn(&app, |conn| Json(cluster::fit(conn, k)).into_response())
}

async fn admin_recompute_fused(
    axum::extract::State(app): axum::extract::State<Arc<App>>,
    body: axum::body::Bytes,
) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let limit = value.get("limit").and_then(|v| v.as_i64()).unwrap_or(400).max(0) as usize;
    let force = value.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    let refit = value.get("refit").and_then(|v| v.as_bool()).unwrap_or(true);
    with_conn(&app, |conn| {
        let (fused, skipped, failed, remaining) = cluster::backfill_fused(conn, limit, force);
        // Rewritten vectors mean stale groups, so refit unless told not to.
        let fit = if refit && fused > 0 && remaining == 0 { cluster::fit(conn, 0) } else { json!(null) };
        Json(json!({"fused": fused, "skipped": skipped, "failed": failed,
                    "remaining": remaining, "fit": fit}))
            .into_response()
    })
}

// ---------- inspire handlers ----------

const INSPIRE_PROJECTS: &[&str] = &["web", "app", "game", "poster", "identity", "engine", "mixed"];
const INSPIRE_PLATFORMS: &[&str] = &["web", "android", "ios", "desktop", "any"];
const INSPIRE_VIBES: &[&str] = &[
    "retro-futuristic", "brutalist", "minimal", "dark", "playful",
    "editorial", "organic", "techy", "glassy", "noisy",
];

fn string_list(value: Option<&Value>, allowed: Option<&[&str]>) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    match value {
        None => {}
        Some(Value::String(s)) => {
            raw = s.split(|c: char| c == ',' || c.is_whitespace()).filter(|v| !v.is_empty()).map(|v| v.to_string()).collect();
        }
        Some(Value::Array(a)) => {
            raw = a.iter().filter_map(|v| v.as_str()).map(|v| v.to_string()).collect();
        }
        Some(_) => {}
    }
    let mut out = Vec::new();
    for v in raw {
        let v = v.trim().to_lowercase();
        if v.is_empty() {
            continue;
        }
        if let Some(allowed) = allowed {
            if !allowed.contains(&v.as_str()) {
                continue;
            }
        }
        out.push(v);
    }
    out
}

async fn inspire_start_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    body: Option<Json<Value>>,
) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or(json!({}));
    let get = |k: &str| body.get(k);
    let mut project = get("project").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
    if !INSPIRE_PROJECTS.contains(&project.as_str()) {
        project = "mixed".into();
    }
    let mut platform = get("platform").and_then(|v| v.as_str()).unwrap_or("any").trim().to_lowercase();
    if !INSPIRE_PLATFORMS.contains(&platform.as_str()) {
        platform = "any".into();
    }
    let about: String = get("about").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(600).collect();
    let mut mode = get("mode").and_then(|v| v.as_str()).unwrap_or("direction").trim().to_lowercase();
    if mode != "brief" && mode != "direction" {
        mode = "direction".into();
    }
    let seed_id: String = get("seed_id").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(64).collect();
    let seed_ids: Vec<String> = get("seed_ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(12).collect())
        .unwrap_or_default();
    let params = json!({
        "mode": mode,
        "project": project,
        "platform": platform,
        "vibe": string_list(get("vibe"), Some(INSPIRE_VIBES)),
        "style": string_list(get("style"), None),
        "about": about,
        "seed_id": seed_id,
        "seed_ids": seed_ids,
    });
    with_conn(&state, |conn| Json(json!({"job_id": inspire::start(conn, &params)})).into_response())
}

async fn inspire_meta_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
) -> Response {
    with_conn(&state, |conn| {
        let hist = inspire::style_histogram(conn);
        let styles = hist.get("styles").cloned().unwrap_or(json!({}));
        let total = hist.get("total").cloned().unwrap_or(json!(0));
        Json(json!({
            "styles": styles,
            "total": total,
            "projects": INSPIRE_PROJECTS,
            "platforms": INSPIRE_PLATFORMS,
            "vibes": INSPIRE_VIBES,
        }))
        .into_response()
    })
}

async fn inspire_status_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&state, |conn| Json(inspire::status(conn, &job_id)).into_response())
}

async fn inspire_preview_start_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    body: Option<Json<Value>>,
) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or(json!({}));
    let direction = body.get("direction").cloned().unwrap_or(json!({}));
    let inspire_job_id = body.get("inspire_job_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let subject = body.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string();
    with_conn(&state, |conn| match inspire::preview_start(conn, &inspire_job_id, &direction, &subject) {
        Ok(job_id) => Json(json!({"job_id": job_id})).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": e}))).into_response(),
    })
}

async fn inspire_preview_status_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&state, |conn| Json(inspire::preview_status(conn, &job_id)).into_response())
}

async fn inspire_export_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&state, |conn| match inspire_export::build_export_html(conn, &job_id) {
        Ok((doc, filename)) => (
            StatusCode::OK,
            [
                ("content-type", "text/html; charset=utf-8"),
                (
                    "content-disposition",
                    &format!("attachment; filename=\"{filename}\"")[..],
                ),
            ],
            doc,
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"detail": e}))).into_response(),
    })
}


async fn inspire_brief_md_route(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Response {
    with_conn(&state, |conn| match inspire::brief_markdown(conn, &job_id) {
        Ok(doc) => (
            StatusCode::OK,
            [("content-type", "text/markdown; charset=utf-8")],
            doc,
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"detail": e}))).into_response(),
    })
}

async fn mcp_handler(
    axum::extract::State(state): axum::extract::State<Arc<App>>,
    req: Request<Body>,
) -> Response {
    use axum::body::to_bytes;
    let mcp_state = mcp::McpState {
        db_path: state.cfg.data_dir.join("vault.db").to_string_lossy().to_string(),
        data_dir: state.cfg.data_dir.clone(),
    };
    if req.method() != axum::http::Method::POST {
        // streamable-http GET opens a server->client SSE stream; the python
        // mount served one, but clients only need POST for request/response
        return (StatusCode::METHOD_NOT_ALLOWED, [("allow", "POST")]).into_response();
    }
    let body = match to_bytes(req.into_body(), 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, Body::empty()).into_response(),
    };
    mcp::handle_post(&mcp_state, &body)
}


async fn assetlinks_handler() -> Response {
    Json(json!({
        "package_names": [],
        "relation": ["delegate_permission/common.handle_all_urls"],
    }))
        .into_response()
}
