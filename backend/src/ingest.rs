//! URL ingestion — port of ingest.py: enrich (yt-dlp -> OG scrape -> platform
//! fallbacks), reddit via Arctic Shift API + old.reddit HTML fallback,
//! link-thumb service calls, IG embed-page scraping, outlinks_from_text.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::mediaio;

fn http_get(url: &str, timeout_secs: u64, extra_headers: &[(&str, &str)]) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .redirects(10)
        .user_agent(mediaio::UA)
        .build();
    let mut req = agent.get(url);
    for (k, v) in extra_headers {
        req = req.set(k, v);
    }
    let resp = req.call().map_err(|e| e.to_string())?;
    let mut body = String::new();
    use std::io::Read;
    std::io::Read::read_to_string(&mut resp.into_reader(), &mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn host_of(url: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"https?://(?:www\.|m\.|mobile\.)?([^/:?#]+)").unwrap());
    re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_lowercase())
        .unwrap_or_default()
}

// ---------- outlinks (shopping / app stores / short links) ----------

const SHOPPING_HOSTS: &[&str] = &[
    "amazon.com", "amazon.co.uk", "amazon.ca", "amazon.de", "amazon.in",
    "amazon.co.jp", "amazon.com.au", "amazon.fr", "amazon.it", "amazon.es",
    "amazon.nl", "amazon.sg", "amazon.ae", "a.co", "amzn.to",
    "alibaba.com", "1688.com", "aliexpress.com", "aliexpress.us",
    "temu.com", "seeedstudio.com", "seeed.cc", "seedstudio.com",
    "adafruit.com", "sparkfun.com", "digikey.com", "mouser.com",
    "newark.com", "element14.com", "walmart.com", "target.com", "ebay.com",
    "etsy.com", "bestbuy.com", "bhphotovideo.com", "adorama.com",
    "newegg.com", "microcenter.com", "homedepot.com", "lowes.com",
    "ikea.com", "wayfair.com", "costco.com", "crutchfield.com",
    "sweetwater.com", "rei.com", "backcountry.com", "shop.app",
    "apps.apple.com", "itunes.apple.com", "play.google.com",
];

pub const APP_STORE_HOSTS: &[&str] = &["apps.apple.com", "itunes.apple.com", "play.google.com"];

const SHORT_HOSTS: &[&str] = &["t.co", "bit.ly", "goo.gl", "tinyurl.com", "reuters.com"];

fn is_shopping_host(host: &str) -> bool {
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    SHOPPING_HOSTS
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

fn store_kind_for_host(host: &str) -> String {
    let h = host.to_lowercase();
    let h = h.strip_prefix("www.").unwrap_or(&h).to_string();
    if h == "a.co" || h == "amzn.to" || h.starts_with("amazon.") {
        return "amazon".into();
    }
    for (kind, domains) in [
        ("alibaba", vec!["alibaba.com", "1688.com"]),
        ("aliexpress", vec!["aliexpress.com", "aliexpress.us"]),
        ("temu", vec!["temu.com"]),
        ("seeed", vec!["seeedstudio.com", "seeed.cc", "seedstudio.com"]),
    ] {
        if domains.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))) {
            return kind.into();
        }
    }
    "shopping".into()
}

fn re_once(key: &'static str, pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<HashMap<&'static str, Regex>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        let mut m = HashMap::new();
        let specs: [(&'static str, &'static str); 8] = [
            ("store_path", r"/(?:dp/|gp/product/|product-detail(?:/|[-_])|products?/|item(?:/|[-_])|itm/|goods(?:/|[-_])|listing/|sku/|shop/.|buy/|store/.|p/|d/)"),
            ("store_query", r"(?:asin|sku|item[_-]?id|product[_-]?id|goods[_-]?id)="),
            ("generic_store_path", r"/(?:product(?:s)?|product-detail|item|itm|goods|listing|sku|shop|buy|store)(?:/|[-_])"),
            ("shopping_action", r"(?:buy|shop|purchase|order|add\s+to\s+cart|get\s+yours|available\s+here)"),
            ("url", r#"(?i)https?://[^\s<>"']+"#),
            ("reddit_post", r"(?i)/comments/([A-Za-z0-9]+)"),
            ("reddit_share", r"(?i)reddit\.com/(?:r/[^/]+/)?s/[A-Za-z0-9]+"),
            ("reddit_short", r"(?i)(?:^|[^.\w])redd\.it/[A-Za-z0-9]+"),
        ];
        for (k, p) in specs {
            m.insert(k, Regex::new(p).unwrap());
        }
        m
    });
    map.get(key).unwrap()
}

pub fn is_shopping_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let host = parsed.host_str().unwrap_or("").to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() || !(parsed.scheme() == "http" || parsed.scheme() == "https") {
        return false;
    }
    let path = parsed.path().to_lowercase();
    let path = path.trim_end_matches('/').to_string();
    if path.is_empty() && parsed.query().is_none() {
        return false;
    }
    let query = parsed.query().unwrap_or("");
    let known_store = is_shopping_host(&host);
    let strong_query = re_once("store_query", "").is_match(query);
    let strong_path = re_once("store_path", "").is_match(&path);
    let generic_path = re_once("generic_store_path", "").is_match(&path);
    let shared_product = (host.ends_with(".aliexpress.com")
        && (path.starts_with("/_") || path.starts_with("/e/")))
        || (host.ends_with(".temu.com") && host.starts_with("share.") && !path.is_empty());
    if strong_query || shared_product {
        return known_store;
    }
    if strong_path {
        return known_store;
    }
    if generic_path && path.trim_matches('/').len() > 8 {
        let _ = re_once("shopping_action", ""); // referenced for parity
        return known_store;
    }
    false
}

fn expand_short(url: &str) -> String {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .redirects(3)
        .build();
    match agent.get(url).call() {
        Ok(resp) => resp.get_url().split('?').next().unwrap_or("").to_string(),
        Err(_) => String::new(),
    }
}

fn is_outlink(host: &str) -> bool {
    !host.is_empty() && !host.contains("taste-vault")
}

fn app_store_meta(url: &str) -> Map<String, Value> {
    let host = host_of(url);
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let mut out = Map::new();
    if !APP_STORE_HOSTS.contains(&host.as_str()) {
        return out;
    }
    let parsed = url::Url::parse(url).ok();
    let path = parsed
        .as_ref()
        .map(|p| percent_decode(p.path()).to_lowercase())
        .unwrap_or_default();
    let path = path.trim_end_matches('/').to_string();
    let slug = path.rsplit('/').next().unwrap_or("").to_string();
    if host == "apps.apple.com" || host == "itunes.apple.com" {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let mut name = String::new();
        if let Some(i) = parts.iter().position(|p| *p == "app") {
            if i + 1 < parts.len() {
                name = parts[i + 1].to_string();
            }
        }
        if name.is_empty() && slug.starts_with("id") {
            name = slug;
        }
        out.insert("kind".into(), json!("apple-app-store"));
        if !name.is_empty() {
            name = name.replace('-', " ");
        }
        out.insert("title".into(), json!(if name.is_empty() { "app store app" } else { &name }));
        out.insert("app_store_platform".into(), json!("apple"));
        return out;
    }
    if host == "play.google.com" {
        let pid = parsed
            .as_ref()
            .and_then(|p| p.query())
            .and_then(|q| {
                q.split('&').find_map(|pair| {
                    let mut kv = pair.splitn(2, '=');
                    let k = kv.next()?;
                    let v = kv.next()?;
                    if k == "id" { Some(v.to_string()) } else { None }
                })
            })
            .unwrap_or_default();
        out.insert("kind".into(), json!("google-play"));
        out.insert("title".into(), json!(if pid.is_empty() { "google play app" } else { &pid }));
        out.insert("app_store_platform".into(), json!("google"));
    }
    out
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.as_bytes())
        .map(|(k, _)| k.to_string())
        .next()
        .unwrap_or_else(|| s.to_string())
        .replace('%', "%")
        .to_string()
}

fn simple_unquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b as char);
                i += 3;
                continue;
            }
        }
        out.push(s[i..].chars().next().unwrap());
        i += s[i..].chars().next().unwrap().len_utf8();
    }
    out
}

/// ingest.outlinks_from_text
pub fn outlinks_from_text(text: &str, title: &str, exclude_url: &str) -> Vec<Value> {
    let haystack = [title, text].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    if haystack.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(String, Value)> = Vec::new();
    for raw_match in re_once("url", "").find_iter(&haystack) {
        let mut raw = raw_match.as_str().to_string();
        while raw.ends_with(['.', ',', ';', ':', '!', '?']) {
            raw.pop();
        }
        let host = host_of(&raw);
        if host.is_empty() {
            continue;
        }
        let final_url = if SHORT_HOSTS.contains(&host.as_str()) {
            let e = expand_short(&raw);
            if e.is_empty() {
                continue;
            }
            e
        } else {
            raw.clone()
        };
        let fhost = host_of(&final_url);
        if SHORT_HOSTS.contains(&fhost.as_str()) || !is_outlink(&fhost) {
            continue;
        }
        let link = final_url.trim_end_matches('/').to_string();
        if !exclude_url.is_empty() && (link == exclude_url || exclude_url.contains(&link)) {
            continue;
        }
        if found.iter().any(|(l, _)| l == &link) {
            continue;
        }
        let mut entry = json!({"url": link, "title": ""});
        if APP_STORE_HOSTS.contains(&fhost.as_str()) {
            let meta = app_store_meta(&link);
            let kind = meta.get("kind").and_then(|v| v.as_str()).unwrap_or("app-store").to_string();
            let title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("app").to_string();
            let title = if !title.to_lowercase().ends_with("app") { format!("{title} app") } else { title };
            entry.as_object_mut().unwrap().insert("kind".into(), json!(kind));
            entry.as_object_mut().unwrap().insert("title".into(), json!(title));
            if let Some(p) = meta.get("app_store_platform") {
                entry.as_object_mut().unwrap().insert("app_store_platform".into(), p.clone());
            }
        } else if is_shopping_host(&fhost) && is_shopping_url(&link) {
            entry.as_object_mut().unwrap().insert("kind".into(), json!(store_kind_for_host(&fhost)));
            entry.as_object_mut().unwrap().insert("title".into(), json!("online store product"));
        } else if is_shopping_host(&fhost) {
            entry.as_object_mut().unwrap().insert("kind".into(), json!("web"));
        } else {
            continue;
        }
        found.push((link, entry));
    }
    found.into_iter().map(|(_, v)| v).collect()
}

// ---------- yt-dlp ----------

pub fn ytdlp_metadata(url: &str) -> Value {
    let out = std::process::Command::new("yt-dlp")
        .args(["--no-warnings", "--skip-download", "--dump-single-json", "--socket-timeout", "25"])
        .arg(url)
        .output();
    let Ok(o) = out else { return json!({}) };
    if !o.status.success() {
        return json!({});
    }
    let Ok(info) = serde_json::from_slice::<Value>(&o.stdout) else { return json!({}) };
    let mut images: Vec<Value> = Vec::new();
    if let Some(thumb) = info.get("thumbnail").and_then(|v| v.as_str()) {
        images.push(json!(thumb));
    }
    let mut video = String::new();
    for fmt in info.get("formats").and_then(|f| f.as_array()).into_iter().flatten() {
        let vcodec = fmt.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");
        let fmt_url = fmt.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if vcodec != "none" && !fmt_url.is_empty() {
            video = fmt_url.to_string();
            break;
        }
    }
    let uploader = info.get("uploader").and_then(|v| v.as_str()).unwrap_or("");
    let author = if uploader.is_empty() {
        info.get("channel").and_then(|v| v.as_str()).unwrap_or("")
    } else {
        uploader
    };
    json!({
        "title": info.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        "text": info.get("description").and_then(|v| v.as_str()).unwrap_or("").chars().take(4000).collect::<String>(),
        "author": author,
        "created_at": "",
        "images": images,
        "video": video,
    })
}

// ---------- reddit ----------

const ARCTIC_SHIFT: &str = "https://arctic-shift.photon-reddit.com";
const REDDIT_COMMENTS_MARK: &str = "--- community comments (ranked by score) ---";

pub fn reddit_post_id(url: &str) -> String {
    re_once("reddit_post", "").captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default()
}

pub fn reddit_resolve(url: &str) -> String {
    let raw = url.trim();
    if raw.is_empty()
        || (!re_once("reddit_share", "").is_match(raw) && !re_once("reddit_short", "").is_match(raw))
    {
        return String::new();
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(20))
        .redirects(10)
        .user_agent(mediaio::UA)
        .build();
    if let Ok(resp) = agent.get(raw).call() {
        let final_url = resp.get_url().split('?').next().unwrap_or("").to_string();
        if !reddit_post_id(&final_url).is_empty() {
            return final_url;
        }
    }
    String::new()
}

fn arctic_section(path: &str, params: &[(&str, &str)]) -> Vec<Value> {
    let mut url = format!("{ARCTIC_SHIFT}{path}");
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlquery_encode(v)))
        .collect();
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    let body = match http_get(&url, 30, &[]) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let Ok(v) = serde_json::from_str::<Value>(&body) else { return Vec::new() };
    v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default()
}

fn urlquery_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn reddit_clean(value: &str) -> String {
    let v = value.replace("\r\n", "\n").replace('\r', "\n");
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\n{3,}").unwrap());
    re.replace_all(&v, "\n\n").trim().to_string()
}

fn arctic_media(post: &Value) -> (Vec<String>, String) {
    let mut imgs: Vec<String> = Vec::new();
    let mut video = String::new();
    let ext_map: HashMap<&str, &str> = HashMap::from([
        ("image/jpeg", "jpg"), ("image/png", "png"), ("image/gif", "gif"),
        ("image/webp", "webp"), ("image/avif", "avif"),
    ]);
    if let Some(meta) = post.get("media_metadata").and_then(|m| m.as_object()) {
        for (_k, item) in meta {
            if item.get("status").and_then(|v| v.as_str()) != Some("valid") {
                continue;
            }
            let ext = ext_map.get(item.get("m").and_then(|v| v.as_str()).unwrap_or("")).copied();
            let mid = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(ext) = ext {
                if !mid.is_empty() {
                    imgs.push(format!("https://i.redd.it/{mid}.{ext}"));
                    continue;
                }
            }
            let src = item
                .get("s")
                .and_then(|s| s.get("u"))
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .replace("&amp;", "&");
            if !src.is_empty() {
                imgs.push(src);
            }
        }
    }
    if imgs.is_empty() {
        if let Some(preview) = post.get("preview").and_then(|p| p.get("images")).and_then(|i| i.as_array()) {
            for im in preview {
                let src = im
                    .get("source")
                    .and_then(|s| s.get("url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .replace("&amp;", "&");
                if !src.is_empty() {
                    imgs.push(src);
                }
            }
        }
    }
    let uod = post.get("url_overridden_by_dest").and_then(|v| v.as_str()).unwrap_or("");
    if imgs.is_empty() {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"(?i)\.(?:jpe?g|png|gif|webp|avif)(?:\?|$)").unwrap());
        if re.is_match(uod) {
            imgs.push(uod.to_string());
        }
    }
    if let Some(media_field) = post.get("media").and_then(|m| m.as_object()) {
        video = media_field
            .get("reddit_video")
            .and_then(|r| r.get("fallback_url"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
    }
    if video.is_empty() {
        if let Some(sm) = post.get("secure_media").and_then(|m| m.as_object()) {
            video = sm
                .get("reddit_video")
                .and_then(|r| r.get("fallback_url"))
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
        }
    }
    (imgs.into_iter().take(8).collect(), video.replace("&amp;", "&"))
}

fn reddit_comments_block(comments: &[Value], cap: usize) -> String {
    struct Row {
        score: i64,
        who: String,
        body: String,
    }
    let mut rows: Vec<Row> = Vec::new();
    for c in comments {
        let body = reddit_clean(c.get("body").and_then(|v| v.as_str()).unwrap_or(""));
        if body.is_empty() || body == "[deleted]" || body == "[removed]" {
            continue;
        }
        if c.get("author").and_then(|v| v.as_str()) == Some("AutoModerator") {
            continue;
        }
        let mut who = c.get("author").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        if c.get("is_submitter").and_then(|v| v.as_bool()).unwrap_or(false) {
            who.push_str(" (OP)");
        }
        let score = c.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
        let body_flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
        rows.push(Row { score, who, body: body_flat });
    }
    rows.sort_by(|a, b| b.score.cmp(&a.score)); // stable: ties keep api order
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for r in rows {
        let line = format!("[{} pts | {}] {}", r.score, r.who, r.body);
        if used + line.len() > cap {
            break;
        }
        used += line.len() + 1;
        lines.push(line);
    }
    lines.join("\n")
}

pub fn reddit_meta(url: &str) -> Value {
    let mut out = json!({
        "title": "", "text": "", "author": "", "images": [], "video": "",
        "created_at": "", "resolved_url": ""
    });
    let mut canonical = url.trim().to_string();
    let mut pid = reddit_post_id(&canonical);
    if pid.is_empty() {
        let resolved = reddit_resolve(&canonical);
        if !resolved.is_empty() {
            pid = reddit_post_id(&resolved);
            if !pid.is_empty() {
                canonical = resolved.clone();
                out.as_object_mut().unwrap().insert("resolved_url".into(), json!(resolved));
            }
        }
    }
    let text_cap = crate::write::env_num("TV_TEXT_CAP", 16000);
    if !pid.is_empty() {
        let rows = arctic_section("/api/posts/ids", &[("ids", &pid)]);
        let post = rows.first().cloned().unwrap_or(json!({}));
        if post.is_object() {
            let obj = out.as_object_mut().unwrap();
            obj.insert("title".into(), json!(reddit_clean(post.get("title").and_then(|v| v.as_str()).unwrap_or(""))));
            obj.insert("author".into(), json!(post.get("author").and_then(|v| v.as_str()).unwrap_or("").trim()));
            let ts = post.get("created_utc").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if ts > 0.0 {
                obj.insert("created_at".into(), json!(unix_to_iso(ts)));
            }
            let body = reddit_clean(post.get("selftext").and_then(|v| v.as_str()).unwrap_or(""));
            let body: String = body.chars().take(8000).collect();
            let comments = arctic_section(
                "/api/comments/search",
                &[("link_id", &pid), ("limit", "auto"), ("sort", "desc")],
            );
            let room = (text_cap as usize).saturating_sub(body.len()).saturating_sub(REDDIT_COMMENTS_MARK.len()).saturating_sub(4);
            let block = reddit_comments_block(&comments, room);
            let text = match (!body.is_empty(), !block.is_empty()) {
                (true, true) => format!("{body}\n\n{REDDIT_COMMENTS_MARK}\n{block}"),
                (true, false) => body,
                (false, true) => format!("{REDDIT_COMMENTS_MARK}\n{block}"),
                _ => String::new(),
            };
            obj.insert("text".into(), json!(text));
            let (images, video) = arctic_media(&post);
            obj.insert("images".into(), json!(images));
            obj.insert("video".into(), json!(video));
        }
    }
    let empty_title = out.get("title").and_then(|v| v.as_str()).unwrap_or("").is_empty();
    let empty_text = out.get("text").and_then(|v| v.as_str()).unwrap_or("").is_empty();
    if empty_title || empty_text {
        let html_out = reddit_meta_html(if canonical.is_empty() { url } else { &canonical });
        let obj = out.as_object_mut().unwrap();
        for key in ["title", "text", "author", "created_at", "video"] {
            let cur = obj.get(key).and_then(|v| v.as_str()).unwrap_or("").is_empty();
            if cur {
                if let Some(v) = html_out.get(key) {
                    obj.insert(key.into(), v.clone());
                }
            }
        }
        let no_imgs = {
            let imgs = obj.get("images").cloned().unwrap_or(json!([]));
            imgs.as_array().map(|a| a.is_empty()).unwrap_or(true)
        };
        if no_imgs {
            if let Some(imgs) = html_out.get("images") {
                if !imgs.is_null() {
                    obj.insert("images".into(), imgs.clone());
                }
            }
        }
    }
    out
}

fn unix_to_iso(ts: f64) -> String {
    // "%Y-%m-%dT%H:%M:%S+00:00" in UTC
    let secs = ts as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02}T{h:02}:{m:02}:{s:02}+00:00")
}

fn reddit_meta_html(url: &str) -> Value {
    let out = json!({"title": "", "text": "", "author": "", "images": [], "video": "", "created_at": ""});
    static RE_OLD: OnceLock<Regex> = OnceLock::new();
    let re_old = RE_OLD.get_or_init(|| Regex::new(r"(?i)https?://(?:www\.|old\.|m\.)?reddit\.com").unwrap());
    let old = re_old.replace(url, "https://old.reddit.com").to_string();
    let html = match http_get(&old, 25, &[]) {
        Ok(h) if !h.is_empty() => h,
        _ => return out,
    };
    let obj = out.as_object().cloned().unwrap_or_default();
    let mut out = obj;
    static RE_THING: OnceLock<Regex> = OnceLock::new();
    let re_thing = RE_THING.get_or_init(|| Regex::new(r#"id="thing_t3_([A-Za-z0-9]+)""#).unwrap());
    let (post_scope, pid) = match re_thing.captures(&html) {
        Some(m) => {
            let start = m.get(0).unwrap().start();
            let mut scope = html[start..].to_string();
            let pid = m.get(1).unwrap().as_str().to_string();
            if let Some(cend) = scope.find("class=\"commentarea\"") {
                scope = scope[..cend].to_string();
            }
            (scope, pid)
        }
        None => (html.clone(), String::new()),
    };
    static RE_TITLE: OnceLock<Regex> = OnceLock::new();
    let re_title = RE_TITLE.get_or_init(|| Regex::new(r#"(?s)<a class="title may-blank[^"]*"[^>]*>(.*?)</a>"#).unwrap());
    let mut title = String::new();
    if let Some(m) = re_title.captures(&post_scope) {
        title = collapse_ws(m.get(1).unwrap().as_str());
        title = html_unescape(&title);
    }
    if title.is_empty() {
        static RE_OGT: OnceLock<Regex> = OnceLock::new();
        let re_ogt = RE_OGT.get_or_init(|| Regex::new(r#"property="og:title"[^>]*content="([^"]+)""#).unwrap());
        if let Some(m) = re_ogt.captures(&html) {
            title = html_unescape(&collapse_ws(m.get(1).unwrap().as_str()));
        }
    }
    if !title.is_empty() {
        out.insert("title".into(), json!(title));
    }
    let mut md: Option<String> = None;
    if !pid.is_empty() {
        let pat = format!(r#"(?s)name="thing_id" value="t3_{}".*?<div class="md">(.*?)</div>"#, regex_escape(&pid));
        if let Ok(re) = Regex::new(&pat) {
            if let Some(m) = re.captures(&post_scope) {
                md = Some(m.get(1).unwrap().as_str().to_string());
            }
        }
    }
    if md.is_none() {
        static RE_MD: OnceLock<Regex> = OnceLock::new();
        let re_md = RE_MD.get_or_init(|| Regex::new(r#"(?s)<div class="md">(.*?)</div>"#).unwrap());
        if let Some(m) = re_md.captures(&post_scope) {
            md = Some(m.get(1).unwrap().as_str().to_string());
        }
    }
    if md.is_none() {
        static RE_OGD: OnceLock<Regex> = OnceLock::new();
        let re_ogd = RE_OGD.get_or_init(|| Regex::new(r#"property="og:description"[^>]*content="([^"]+)""#).unwrap());
        if let Some(m) = re_ogd.captures(&html) {
            md = Some(m.get(1).unwrap().as_str().to_string());
        }
    }
    if let Some(md) = md {
        static RE_TAGS: OnceLock<Regex> = OnceLock::new();
        static RE_MD_LINK: OnceLock<Regex> = OnceLock::new();
        static RE_MD_JUNK: OnceLock<Regex> = OnceLock::new();
        let re_tags = RE_TAGS.get_or_init(|| Regex::new(r"<[^>]+>").unwrap());
        let re_mlink = RE_MD_LINK.get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());
        let re_junk = RE_MD_JUNK.get_or_init(|| Regex::new(r"[#*`>]+").unwrap());
        let mut text = re_tags.replace_all(&md, " ").to_string();
        text = html_unescape(&text);
        text = re_mlink.replace_all(&text, "$1").to_string();
        text = re_junk.replace_all(&text, " ").to_string();
        text = collapse_ws(&text).trim().to_string();
        let text: String = text.chars().take(4000).collect();
        out.insert("text".into(), json!(text));
    }
    static RE_AUTHOR: OnceLock<Regex> = OnceLock::new();
    let re_author = RE_AUTHOR.get_or_init(|| Regex::new(r#"<a[^>]*class="author[^"]*"[^>]*>([^<]+)</a>"#).unwrap());
    if let Some(m) = re_author.captures(&post_scope) {
        out.insert("author".into(), json!(m.get(1).unwrap().as_str().trim()));
    }
    static RE_TIME: OnceLock<Regex> = OnceLock::new();
    let re_time = RE_TIME.get_or_init(|| Regex::new(r#"<time[^>]*datetime="([^"]+)""#).unwrap());
    if let Some(m) = re_time.captures(&post_scope) {
        let created: String = m.get(1).unwrap().as_str().trim().chars().take(40).collect();
        out.insert("created_at".into(), json!(created));
    }
    static REDDIT_IMGS: OnceLock<Regex> = OnceLock::new();
    let re_imgs = REDDIT_IMGS.get_or_init(|| Regex::new(r#"https://(?:i|preview|external-preview)\.redd\.it/[^"'s<>]+"#).unwrap());
    let mut uniq: Vec<String> = Vec::new();
    for m in re_imgs.find_iter(&post_scope) {
        let mut u = simple_unquote(m.as_str());
        u = u.replace("&amp;", "&");
        let low = u.to_lowercase();
        if ["avatar", "emoji", "flutter", "spacer"].iter().any(|x| low.contains(x)) {
            continue;
        }
        if !uniq.contains(&u) {
            uniq.push(u);
        }
    }
    // keep the largest width variant per base path
    let mut best_by_path: HashMap<String, (String, i64)> = HashMap::new();
    for u in &uniq {
        let key = url::Url::parse(u).map(|p| p.path().to_string()).unwrap_or_else(|_| u.clone());
        let mut w = 0i64;
        if let Ok(p) = url::Url::parse(u) {
            if let Some(q) = p.query() {
                for pair in q.split('&') {
                    let mut kv = pair.splitn(2, '=');
                    let k = kv.next().unwrap_or("");
                    let v = kv.next().unwrap_or("");
                    if k == "width" {
                        w = v.parse::<i64>().unwrap_or(0);
                    }
                }
            }
        }
        match best_by_path.get(&key) {
            None => {
                best_by_path.insert(key, (u.clone(), w));
            }
            Some((_cu, cw)) if w > *cw => {
                best_by_path.insert(key, (u.clone(), w));
            }
            _ => {}
        }
    }
    let mut best: Vec<(String, i64)> = best_by_path.into_values().collect();
    best.sort_by(|a, b| b.1.cmp(&a.1));
    let uniq: Vec<String> = best.into_iter().map(|(u, _)| u).take(8).collect();
    if !uniq.is_empty() {
        out.insert("images".into(), json!(uniq));
    } else {
        static RE_OGI: OnceLock<Regex> = OnceLock::new();
        let re_ogi = RE_OGI.get_or_init(|| Regex::new(r#"property="og:image"[^>]*content="([^"]+)""#).unwrap());
        if let Some(m) = re_ogi.captures(&html) {
            let u = m.get(1).unwrap().as_str().replace("&amp;", "&");
            if u.contains("redd.it") {
                out.insert("images".into(), json!([u]));
            }
        }
    }
    static RE_VREDD: OnceLock<Regex> = OnceLock::new();
    let re_vredd = RE_VREDD.get_or_init(|| Regex::new(r"https://v\.redd\.it/[A-Za-z0-9]+").unwrap());
    if let Some(m) = re_vredd.captures(&post_scope) {
        out.insert("video".into(), json!(m.get(0).unwrap().as_str()));
    }
    Value::Object(out)
}

fn regex_escape(s: &str) -> String {
    regex::escape(s)
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn html_unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = value[i..].find(';') {
                if semi <= 10 {
                    let ent = &value[i + 1..i + semi];
                    if let Some(d) = crate::vault::decode_entity_pub(ent) {
                        out.push_str(&d);
                        i += semi + 1;
                        continue;
                    }
                }
            }
        }
        let ch = value[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

// ---------- og scrape ----------

pub fn scrape_og(url: &str) -> Value {
    let mut out = json!({"title": "", "text": "", "author": "", "images": [], "video": "", "created_at": ""});
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(20))
        .redirects(10)
        .user_agent(mediaio::UA)
        .build();
    let html = match agent.get(url).call() {
        Ok(resp) => {
            let mut body = String::new();
            use std::io::Read;
            let _ = std::io::Read::read_to_string(&mut resp.into_reader(), &mut body);
            body
        }
        Err(_) => return out,
    };
    if html.is_empty() {
        return out;
    }
    if crate::vault::platform_of_pub(url) == "instagram"
        && (Regex::new(r"(?i)PolarisLoggedOutPrivate(?:Desktop)?PostRoute|This Account is Private|account is private").unwrap().is_match(&html)
            || (html.contains("PolarisLoggedOutCallToAction.react")
                && !Regex::new(r#"(?i)property=["']og:image"#).unwrap().is_match(&html)))
    {
        return json!({
            "title": "Instagram post", "text": "Account private", "author": "",
            "images": [], "video": "", "created_at": "", "private": true
        });
    }
    let metas = |prop: &str| -> Vec<String> {
        let esc = regex_escape(prop);
        let re_a = Regex::new(&format!(
            r#"<meta[^>]+(?:property|name)=["']{esc}["'][^>]+content=["']([^"']+)["']"#
        ))
        .unwrap();
        let re_b = Regex::new(&format!(
            r#"<meta[^>]+content=["']([^"']+)["'][^>]+(?:property|name)=["']{esc}["']"#
        ))
        .unwrap();
        let found: Vec<String> = re_a
            .captures_iter(&html)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .collect();
        if !found.is_empty() {
            return found;
        }
        re_b
            .captures_iter(&html)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .collect()
    };
    let obj = out.as_object_mut().unwrap();
    let title = metas("og:title")
        .into_iter()
        .chain(metas("twitter:title"))
        .next()
        .unwrap_or_default();
    obj.insert("title".into(), json!(title));
    let text = metas("og:description")
        .into_iter()
        .chain(metas("twitter:description"))
        .next()
        .map(|t| t.chars().take(4000).collect::<String>())
        .unwrap_or_default();
    obj.insert("text".into(), json!(text));
    let mut images: Vec<String> = Vec::new();
    for img in metas("og:image").into_iter().chain(metas("twitter:image")) {
        if !images.contains(&img) {
            images.push(img);
        }
    }
    obj.insert("images".into(), json!(images));
    let vids: Vec<String> = metas("og:video")
        .into_iter()
        .chain(metas("og:video:secure_url"))
        .chain(metas("twitter:player:stream"))
        .collect();
    if let Some(v) = vids.first() {
        obj.insert("video".into(), json!(v));
    }
    if let Some(site) = metas("og:site_name").first() {
        obj.insert("author".into(), json!(site));
    }
    let title_empty = obj.get("title").and_then(|v| v.as_str()).unwrap_or("").is_empty();
    if title_empty {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
        if let Some(m) = re.captures(&html) {
            let t = collapse_ws(m.get(1).unwrap().as_str()).trim().to_string();
            let t: String = t.chars().take(300).collect();
            obj.insert("title".into(), json!(t));
        }
    }
    out
}

// ---------- link-thumb service ----------

fn thumb_svc() -> String {
    std::env::var("THUMB_SVC").unwrap_or_else(|_| "http://localhost:8771".into())
}

fn dedupe_query(url: &str) -> String {
    let parsed = match url::Url::parse(url) {
        Ok(p) => p,
        Err(_) => return url.to_string(),
    };
    if parsed.query().is_none() || parsed.query().unwrap_or("").is_empty() {
        return url.to_string();
    }
    let mut seen: Vec<String> = Vec::new();
    let mut keep: Vec<String> = Vec::new();
    for pair in parsed.query().unwrap_or("").split('&') {
        let key = pair.split('=').next().unwrap_or("").to_string();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        keep.push(pair.to_string());
    }
    let mut rebuilt = format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or(""),
        parsed.path()
    );
    if !keep.is_empty() {
        rebuilt.push('?');
        rebuilt.push_str(&keep.join("&"));
    }
    rebuilt
}

fn linkthumb_get(endpoint: &str, url: &str) -> Option<Value> {
    let req_url = format!("{}{}?url={}", thumb_svc(), endpoint, urlquery_encode(url));
    let body = http_get(&req_url, 25, &[]).ok()?;
    serde_json::from_str::<Value>(&body).ok()
}

pub fn linkthumb_image(url: &str) -> String {
    match linkthumb_get("/json", url) {
        Some(data) => dedupe_query(data.get("image").and_then(|v| v.as_str()).unwrap_or("")),
        None => String::new(),
    }
}

pub fn linkthumb_images(url: &str) -> Vec<String> {
    match linkthumb_get("/images", url) {
        Some(data) => data
            .get("images")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|u| dedupe_query(u))
                    .filter(|u| !u.is_empty())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

pub fn linkthumb_outlinks(url: &str) -> Vec<Value> {
    let req_url = format!("{}{}?url={}", thumb_svc(), "/outlinks", urlquery_encode(url));
    let body = match http_get(&req_url, 25, &[]) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let Ok(data) = serde_json::from_str::<Value>(&body) else { return Vec::new() };
    data.get("outlinks")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|o| o.is_object()).take(16).cloned().collect())
        .unwrap_or_default()
}

// ---------- instagram helpers (via yt-dlp / embed pages) ----------

pub fn ig_video_url(url: &str) -> String {
    let out = std::process::Command::new("yt-dlp")
        .args(["--no-warnings", "--skip-download", "--no-playlist", "--get-url", "--format", "best[ext=mp4]/best"])
        .arg(url)
        .output()
        .ok();
    if let Some(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let line = line.trim();
            if line.starts_with("http") && line.contains(".mp4") {
                return line.to_string();
            }
        }
    }
    String::new()
}

pub fn ig_music_info(url: &str) -> Value {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"instagram\.com/(?:reel|reels|p|tv)/([A-Za-z0-9_-]+)").unwrap());
    let shortcode = match re.captures(url).and_then(|c| c.get(1)) {
        Some(m) => m.as_str(),
        None => return json!({}),
    };
    for kind in ["reel", "p"] {
        let embed_url = format!("https://www.instagram.com/{kind}/{shortcode}/embed/captioned/");
        let html = match http_get(&embed_url, 20, &[("User-Agent", "Mozilla/5.0")]) {
            Ok(h) if !h.is_empty() => h,
            _ => continue,
        };
        let i = match html.find("music_attribution_info") {
            Some(i) => i,
            None => continue,
        };
        let block = &html[i..(i + 500).min(html.len())];
        let art = Regex::new(r#"artist_name\\?"?\s*:\s*\\?"([^"\\]+)"#).unwrap();
        let song = Regex::new(r#"song_name\\?"?\s*:\s*\\?"([^"\\]+)"#).unwrap();
        let artist = art.captures(block).and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
        let song_t = song.captures(block).and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
        if !song_t.is_empty() || !artist.is_empty() {
            return json!({
                "artist": artist.chars().take(120).collect::<String>(),
                "song": song_t.chars().take(200).collect::<String>(),
            });
        }
    }
    json!({})
}

pub fn ig_embed_slides(url: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"instagram\.com/(?:reel|reels|p|tv)/([A-Za-z0-9_-]+)").unwrap());
    let shortcode = match re.captures(url).and_then(|c| c.get(1)) {
        Some(m) => m.as_str(),
        None => return Vec::new(),
    };
    let embed_url = format!("https://www.instagram.com/p/{shortcode}/embed/captioned/");
    let mut txt = match http_get(&embed_url, 20, &[("User-Agent", "Mozilla/5.0")]) {
        Ok(h) if !h.is_empty() => h,
        _ => return Vec::new(),
    };
    while txt.contains("\\\\") {
        txt = txt.replace("\\\\", "\\");
    }
    txt = txt.replace("\\/", "/");
    let re_disp = Regex::new(r#""display_url"\s*:\s*"([^"]+)""#).unwrap();
    let raw = re_disp
        .captures_iter(&txt)
        .map(|c| c.get(1).unwrap().as_str().to_string());
    let mut out: Vec<String> = Vec::new();
    for u in raw {
        let u = dedupe_query(&u);
        let low = u.to_lowercase();
        let path = low.split('?').next().unwrap_or("");
        if !u.starts_with("http") || out.contains(&u) {
            continue;
        }
        if [".mp4", ".webm", ".mov"].iter().any(|x| path.ends_with(x)) {
            continue;
        }
        if ![".jpg", ".jpeg", ".webp", ".png"].iter().any(|x| path.ends_with(x)) {
            continue;
        }
        out.push(u);
        if out.len() >= 8 {
            break;
        }
    }
    out
}

pub fn ig_video_download(url: &str) -> Vec<u8> {
    let tmp = std::env::temp_dir().join(format!(
        "tv_{}_{}.mp4",
        std::process::id(),
        crate::write::token_hex8()
    ));
    let result = std::process::Command::new("yt-dlp")
        .args(["--no-warnings", "--no-playlist", "--socket-timeout", "15", "-f", "best[ext=mp4]/best", "-o"])
        .arg(&tmp)
        .arg(url)
        .output();
    let data = match result {
        Ok(o) if o.status.success() && tmp.exists() && std::fs::metadata(&tmp).map(|m| m.len() > 20000).unwrap_or(false) => {
            std::fs::read(&tmp).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    let _ = std::fs::remove_file(&tmp);
    data
}

// ---------- enrich ----------

pub fn enrich(url: &str) -> Value {
    let platform = crate::vault::platform_of_pub(url);
    let mut meta = json!({
        "title": "", "text": "", "author": "", "images": [], "video": "",
        "created_at": "", "platform": platform, "private": false, "outlinks": []
    });
    if platform == "reddit" {
        let rm = reddit_meta(url);
        if rm.is_object() {
            let obj = meta.as_object_mut().unwrap();
            for k in ["title", "text", "author", "created_at"] {
                let v = rm.get(k).cloned().unwrap_or(Value::Null);
                let empty = match &v {
                    Value::String(s) => s.is_empty(),
                    Value::Null => true,
                    _ => false,
                };
                if !empty {
                    obj.insert(k.into(), v);
                }
            }
            if let Some(imgs) = rm.get("images") {
                if let Some(arr) = imgs.as_array() {
                    if !arr.is_empty() {
                        let mut uniq: Vec<Value> = Vec::new();
                        for img in arr {
                            if !uniq.contains(img) {
                                uniq.push(img.clone());
                            }
                        }
                        obj.insert("images".into(), json!(uniq));
                    }
                }
            }
            if let Some(v) = rm.get("video") {
                if v.as_str().map(|s| !s.is_empty()).unwrap_or(false) {
                    obj.insert("video".into(), v.clone());
                }
            }
            if let Some(r) = rm.get("resolved_url") {
                if r.as_str().map(|s| !s.is_empty()).unwrap_or(false) {
                    obj.insert("resolved_url".into(), r.clone());
                }
            }
        }
    }
    if ["youtube", "tiktok", "instagram", "pinterest", "reddit", "x", "vimeo"].contains(&platform.as_str()) {
        let yd = ytdlp_metadata(url);
        if yd.is_object() && !yd.as_object().unwrap().is_empty() {
            let has_title = !meta.get("title").and_then(|v| v.as_str()).unwrap_or("").is_empty();
            let obj = meta.as_object_mut().unwrap();
            if platform == "reddit" && has_title {
                for (k, v) in yd.as_object().unwrap() {
                    let truthy = match v {
                        Value::String(s) => !s.is_empty(),
                        Value::Array(a) => !a.is_empty(),
                        Value::Null => false,
                        _ => true,
                    };
                    let empty_key = match obj.get(k) {
                        None => true,
                        Some(Value::String(s)) => s.is_empty(),
                        Some(Value::Array(a)) => a.is_empty(),
                        Some(Value::Null) => true,
                        Some(_) => false,
                    };
                    if truthy && empty_key {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            } else {
                for (k, v) in yd.as_object().unwrap() {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }
    let no_title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").is_empty();
    if no_title || platform == "instagram" {
        let og = scrape_og(url);
        if og.get("private").and_then(|v| v.as_bool()).unwrap_or(false) {
            let has_text = !meta.get("text").and_then(|v| v.as_str()).unwrap_or("").is_empty();
            let has_title = !meta.get("title").and_then(|v| v.as_str()).unwrap_or("").is_empty();
            if !has_text && !has_title {
                let obj = meta.as_object_mut().unwrap();
                for (k, v) in og.as_object().unwrap() {
                    obj.insert(k.clone(), v.clone());
                }
            }
            meta.as_object_mut().unwrap().insert("platform".into(), json!(platform));
            return meta;
        }
        let obj = meta.as_object_mut().unwrap();
        let og_title = og.get("title").and_then(|v| v.as_str()).unwrap_or("");
        if !og_title.is_empty() && obj.get("title").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            obj.insert("title".into(), json!(og_title));
        }
        let og_text = og.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if !og_text.is_empty() && obj.get("text").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            obj.insert("text".into(), json!(og_text));
        }
        if obj.get("author").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            if let Some(a) = og.get("author") {
                obj.insert("author".into(), a.clone());
            }
        }
        if let Some(og_imgs) = og.get("images").and_then(|v| v.as_array()) {
            let imgs = obj.get("images").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let mut all = imgs;
            for img in og_imgs {
                if !all.contains(img) {
                    all.push(img.clone());
                }
            }
            obj.insert("images".into(), json!(all));
        }
        let og_video = og.get("video").and_then(|v| v.as_str()).unwrap_or("");
        if !og_video.is_empty() && obj.get("video").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            obj.insert("video".into(), json!(og_video));
        }
    }
    let text_now = meta.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let title_now = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let obj = meta.as_object_mut().unwrap();
    if platform == "x" {
        let imgs = linkthumb_images(url);
        if !imgs.is_empty() {
            obj.insert("images".into(), json!(imgs));
        }
    }
    if platform == "reddit" {
        let empty = obj.get("images").and_then(|v| v.as_array()).map(|a| a.is_empty()).unwrap_or(true);
        if empty {
            let imgs = linkthumb_images(url);
            if !imgs.is_empty() {
                obj.insert("images".into(), json!(imgs));
            }
        }
    }
    let empty_imgs = obj.get("images").and_then(|v| v.as_array()).map(|a| a.is_empty()).unwrap_or(true);
    if empty_imgs && is_shopping_host(&host_of(url)) {
        let imgs = linkthumb_images(url);
        if !imgs.is_empty() {
            obj.insert("images".into(), json!(imgs));
        }
    }
    if !["youtube", "tiktok", "instagram", "pinterest", "vimeo", "x"].contains(&platform.as_str()) {
        let ol = linkthumb_outlinks(url);
        if !ol.is_empty() {
            obj.insert("outlinks".into(), json!(ol));
        }
    }
    let text_links = outlinks_from_text(
        &text_now,
        &title_now,
        url,
    );
    if !text_links.is_empty() {
        let mut cur: Vec<Value> = obj.get("outlinks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let seen: Vec<String> = cur
            .iter()
            .filter_map(|o| o.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()))
            .collect();
        for o in text_links {
            let o_url = o.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if !seen.contains(&o_url.to_string()) {
                cur.push(o);
            }
        }
        obj.insert("outlinks".into(), json!(cur));
    }
    if platform == "instagram" {
        let music = ig_music_info(url);
        let has = music.get("song").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
            || music.get("artist").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        if has {
            obj.insert("ig_music".into(), music);
        }
    }
    meta
}
