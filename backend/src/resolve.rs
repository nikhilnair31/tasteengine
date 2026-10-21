//! Link resolvers for follow-up rows — ports of songlink.py, showlink.py,
//! gamelink.py, maps.py, applink.py, shopping.py. Each is keyless.

use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

use crate::mediaio::http_agent;

// ---------- maps.py ----------

pub fn maps_place_url(place: &str) -> String {
    let q = place.trim();
    if q.is_empty() {
        return String::new();
    }
    format!(
        "https://www.google.com/maps/search/?api=1&query={}",
        crate::vault::quote_plus_pub(q)
    )
}

// ---------- applink.py ----------

pub fn is_masked_url(url: &str) -> bool {
    // X{4,} anywhere (placeholder app ids)
    let mut run = 0;
    for c in url.chars() {
        if c == 'X' || c == 'x' {
            run += 1;
            if run >= 4 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

pub fn is_app_like(title: &str, why: &str) -> bool {
    let hay = format!("{title} {why}").to_lowercase();
    let words = ["app", "apps", "software", "mobile app", "ios app", "android app", "mac app", "desktop app"];
    for w in words {
        let pat = format!(r"\b{}\b", regex::escape(w));
        if let Ok(re) = regex::Regex::new(&pat) {
            if re.is_match(&hay) {
                return true;
            }
        }
    }
    false
}

pub fn applemac_store_url(title: &str) -> String {
    let name = title.trim();
    if name.is_empty() {
        return String::new();
    }
    let agent = http_agent(Duration::from_secs(12));
    let url = format!(
        "https://itunes.apple.com/search?term={}&entity=software&country=US&limit=5",
        crate::vault::quote_plus_pub(&name.chars().take(120).collect::<String>())
    );
    let Ok(resp) = agent.get(&url).call() else { return String::new() };
    let mut text = String::new();
    use std::io::Read;
    let _ = resp.into_reader().read_to_string(&mut text);
    let Ok(data) = serde_json::from_str::<Value>(&text) else { return String::new() };
    for entry in data.get("results").and_then(|r| r.as_array()).into_iter().flatten() {
        let u = entry
            .get("trackViewUrl")
            .or_else(|| entry.get("collectionViewUrl"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !u.is_empty() {
            return u.to_string();
        }
    }
    String::new()
}

// ---------- songlink.py ----------

pub fn youtube_for(query: &str) -> Value {
    let query = query.trim();
    if query.is_empty() {
        return json!({});
    }
    let out = Command::new("yt-dlp")
        .args([
            "--no-warnings", "--skip-download", "--no-playlist", "--print",
            "%(id)s|%(title)s|%(channel)s",
            &format!("ytsearch1:{query}"),
        ])
        .output();
    let Ok(out) = out else { return json!({}) };
    let line = String::from_utf8_lossy(&out.stdout);
    let first = line.lines().next().unwrap_or("").trim().to_string();
    let Some((vid, rest)) = first.split_once('|') else { return json!({}) };
    let vid = vid.trim();
    if vid.is_empty() {
        return json!({});
    }
    let (title, channel) = rest.split_once('|').unwrap_or((rest, ""));
    json!({
        "url": format!("https://www.youtube.com/watch?v={vid}"),
        "title": title.trim().chars().take(200).collect::<String>(),
        "channel": channel.trim().chars().take(120).collect::<String>(),
    })
}

// ---------- showlink.py ----------

const SHOWLINK_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

const PROVIDER_PATTERNS: &[(&str, &str)] = &[
    ("netflix", r"https://www\.netflix\.com/title/\d+"),
    ("prime video", r#"https://www\.primevideo\.com/[^"'<>]*|https://www\.amazon\.com/[^"'<>]*AmazonVideo[^"'<>]*"#),
    ("max", r#"https://www\.max\.com/[^"'<>]*"#),
    ("hulu", r#"https://www\.hulu\.com/[^"'<>]*"#),
    ("disney+", r#"https://www\.disneyplus\.com/[^"'<>]*"#),
    ("apple tv+", r#"https://tv\.apple\.com/[^"'<>]*"#),
    ("paramount+", r#"https://www\.paramountplus\.com/[^"'<>]*"#),
    ("peacock", r#"https://www\.peacocktv\.com/[^"'<>]*"#),
];

fn words_set(s: &str) -> std::collections::HashSet<String> {
    regex::Regex::new(r"[a-z0-9]+")
        .map(|re| re.find_iter(s).map(|m| m.as_str().to_string()).collect())
        .unwrap_or_default()
}

pub fn justwatch_for(query: &str) -> Value {
    let query = query.trim();
    if query.is_empty() {
        return json!({});
    }
    let agent = http_agent(Duration::from_secs(20));
    let url = format!(
        "https://www.justwatch.com/us/search?q={}",
        crate::vault::quote_plus_pub(query)
    );
    let Ok(resp) = agent.get(&url).set("User-Agent", SHOWLINK_UA).call() else {
        return json!({});
    };
    if resp.status() != 200 {
        return json!({});
    }
    let mut html = String::new();
    use std::io::Read;
    let _ = resp.into_reader().read_to_string(&mut html);
    let re_link = regex::Regex::new(
        r#"(?is)<a[^>]+href="(https://www\.justwatch\.com/us/(?:tv-show|movie)/[^"]+)"[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    let re_strip = regex::Regex::new(r"<[^>]+>|&[a-z]+;").unwrap();
    let mut candidates: Vec<(f64, String, String)> = Vec::new();
    for cap in re_link.captures_iter(&html) {
        let href = cap.get(1).unwrap().as_str().to_string();
        let anchor = re_strip
            .replace_all(cap.get(2).unwrap().as_str(), " ")
            .replace('\u{a0}', " ")
            .trim()
            .to_string();
        let mut score = if href.contains("/tv-show/") { 1.0 } else { 0.5 };
        if anchor.to_lowercase().contains(&query.to_lowercase()) {
            score += 1.0;
        }
        let q = words_set(&query.to_lowercase());
        let slug_words = words_set(&href.rsplit('/').next().unwrap_or("").replace('-', " "));
        let overlap = if q.is_empty() { 0.0 } else { q.intersection(&slug_words).count() as f64 / q.len() as f64 };
        score += overlap;
        candidates.push((score, href, anchor));
    }
    if candidates.is_empty() {
        return json!({});
    }
    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let best_href = candidates.remove(0).1;
    let Ok(page) = agent.get(&best_href).set("User-Agent", SHOWLINK_UA).call() else {
        return json!({ "jw_url": best_href });
    };
    if page.status() != 200 {
        return json!({ "jw_url": best_href });
    }
    let mut page_html = String::new();
    let _ = page.into_reader().read_to_string(&mut page_html);
    let mut out = serde_json::Map::new();
    out.insert("jw_url".into(), json!(best_href));
    let re_imdb = regex::Regex::new(r#""imdbId"\s*:\s*"(tt\d+)""#).unwrap();
    let imdb = re_imdb.captures(&page_html).map(|c| c.get(1).unwrap().as_str().to_string());
    if let Some(id) = &imdb {
        if best_href.contains("/tv-show/") {
            out.insert("stremio".into(), json!(format!("stremio:///detail/series/{id}/{id}")));
        } else {
            out.insert(
                "stremio".into(),
                json!(format!("stremio:///detail/movie/{id}/{id}?autoPlay=true")),
            );
        }
    }
    for (provider, pattern) in PROVIDER_PATTERNS {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(m) = re.find(&page_html) {
                out.insert("provider".into(), json!(provider));
                out.insert("url".into(), json!(m.as_str()));
                break;
            }
        }
    }
    Value::Object(out)
}

// ---------- gamelink.py ----------

pub fn steam_app_url(appid: &str, name: &str) -> String {
    let slug: String = regex::Regex::new(r"[^a-z0-9]+")
        .unwrap()
        .replace_all(&name.to_lowercase(), "-")
        .trim_matches('-')
        .to_string();
    if !slug.is_empty() {
        format!("https://store.steampowered.com/app/{appid}/{slug}/")
    } else {
        format!("https://store.steampowered.com/app/{appid}/")
    }
}

pub fn steam_url_in_text(text: &str) -> String {
    let re = regex::Regex::new(r"(?i)(?:https?://)?(?:store\.)?steampowered\.com/app/(\d+)").unwrap();
    re.captures(text)
        .map(|c| steam_app_url(c.get(1).unwrap().as_str(), ""))
        .unwrap_or_default()
}

pub fn steam_from_x_author(post_url: &str) -> Value {
    let re = regex::Regex::new(r"(?i)(?:x|twitter)\.com/([A-Za-z0-9_]+)/status/").unwrap();
    let Some(cap) = re.captures(post_url) else { return json!({}) };
    let user = cap.get(1).unwrap().as_str();
    let tail = post_url.rsplit('/').next().unwrap_or("").split('?').next().unwrap_or("").trim_matches('/');
    let api = format!("https://api.fxtwitter.com/{user}/status/{tail}");
    let agent = http_agent(Duration::from_secs(15));
    let Ok(resp) = agent.get(&api).set("User-Agent", SHOWLINK_UA).call() else { return json!({}) };
    if resp.status() != 200 {
        return json!({});
    }
    let mut text = String::new();
    use std::io::Read;
    let _ = resp.into_reader().read_to_string(&mut text);
    let Ok(data) = serde_json::from_str::<Value>(&text) else { return json!({}) };
    let author = data.pointer("/tweet/author").cloned().unwrap_or(json!({}));
    let site = author.pointer("/website/url").and_then(|v| v.as_str()).unwrap_or("");
    let desc = author.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let hit = {
        let a = steam_url_in_text(site);
        if a.is_empty() { steam_url_in_text(desc) } else { a }
    };
    let re_app = regex::Regex::new(r"(?i)(?:https?://)?(?:store\.)?steampowered\.com/app/(\d+)").unwrap();
    match re_app.captures(&hit) {
        Some(c) => json!({
            "url": hit,
            "appid": c.get(1).unwrap().as_str(),
            "name": "",
        }),
        None => json!({}),
    }
}

fn steam_norm(text: &str) -> String {
    regex::Regex::new(r"[^a-z0-9]+")
        .unwrap()
        .replace_all(&text.to_lowercase(), " ")
        .trim()
        .to_string()
}

const STOP_CAND: &[&str] = &[
    "the", "a", "an", "of", "in", "on", "for", "to", "and", "or", "review", "essay", "video",
    "vs", "my", "i", "we", "you", "this", "that", "these", "those", "it", "its", "why", "how",
    "what", "playing", "played", "is", "are", "was", "were", "be", "been", "being", "has",
    "have", "had", "do", "does", "did", "will", "would", "can", "could", "not", "no", "yes",
    "so", "such", "very", "really", "most", "more", "best",
];

fn title_candidates(title: &str) -> Vec<String> {
    let re_word = regex::Regex::new(r"[A-Za-z0-9'’]+").unwrap();
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut span: Vec<&str> = Vec::new();
    for word in re_word.find_iter(title) {
        let w = word.as_str();
        if w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            && !STOP_CAND.contains(&w.to_lowercase().as_str())
        {
            span.push(w);
        } else {
            if !span.is_empty() {
                out.push(std::mem::take(&mut span));
            }
        }
    }
    if !span.is_empty() {
        out.push(span);
    }
    let mut cands: Vec<String> = out
        .iter()
        .filter(|s| s.len() >= 2)
        .map(|s| s.join(" "))
        .collect();
    cands.sort_by_key(|b| std::cmp::Reverse(b.len()));
    cands
}

fn steam_for_once(query: &str, relaxed: bool) -> Value {
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&cc=US&l=english",
        crate::vault::quote_plus_pub(query)
    );
    let agent = http_agent(Duration::from_secs(15));
    let Ok(resp) = agent.get(&url).set("User-Agent", SHOWLINK_UA).call() else { return json!({}) };
    if resp.status() != 200 {
        return json!({});
    }
    let mut text = String::new();
    use std::io::Read;
    let _ = resp.into_reader().read_to_string(&mut text);
    let Ok(data) = serde_json::from_str::<Value>(&text) else { return json!({}) };
    let items = data.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    if items.is_empty() {
        return json!({});
    }
    let nq = steam_norm(query);
    let qwords: std::collections::HashSet<String> = nq.split_whitespace().map(|s| s.to_string()).collect();

    let make = |item: &Value| -> Value {
        let appid = item.get("id").map(|v| v.to_string()).unwrap_or_default();
        let appid = appid.trim_matches('"').to_string();
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if appid.is_empty() || name.is_empty() {
            return json!({});
        }
        json!({
            "url": steam_app_url(&appid, &name),
            "appid": appid,
            "name": name.chars().take(200).collect::<String>(),
        })
    };
    // 1. exact name match
    for item in &items {
        if steam_norm(item.get("name").and_then(|v| v.as_str()).unwrap_or("")) == nq {
            let r = make(item);
            if !r.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                return r;
            }
        }
    }
    // 2. fuzzy subset
    let app_items: Vec<&Value> = {
        let apps: Vec<&Value> = items.iter().filter(|i| i.get("type").and_then(|v| v.as_str()) == Some("app")).collect();
        if apps.is_empty() { items.iter().collect() } else { apps }
    };
    for item in &app_items {
        let nwords: std::collections::HashSet<String> = steam_norm(item.get("name").and_then(|v| v.as_str()).unwrap_or(""))
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if !qwords.is_empty() && !nwords.is_empty() && nwords.is_superset(&qwords) {
            let r = make(item);
            if !r.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                return r;
            }
        }
    }
    // 3. relaxed top hit
    if !relaxed {
        return json!({});
    }
    if let Some(item) = app_items.first() {
        let nwords: std::collections::HashSet<String> = steam_norm(item.get("name").and_then(|v| v.as_str()).unwrap_or(""))
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if !qwords.is_empty() && qwords.intersection(&nwords).next().is_some() {
            let r = make(item);
            if !r.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                return r;
            }
        }
    }
    json!({})
}

pub fn steam_for(query: &str) -> Value {
    let query = query.trim();
    if query.is_empty() {
        return json!({});
    }
    let mut hit = steam_for_once(query, true);
    if hit.as_object().map(|o| o.is_empty()).unwrap_or(true) && query.contains("//") {
        let head = query.split("//").next().unwrap_or("").trim();
        hit = steam_for_once(head, true);
    }
    if hit.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        for cand in title_candidates(query) {
            if steam_norm(&cand) == steam_norm(query) {
                continue;
            }
            hit = steam_for_once(&cand, false);
            if !hit.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                break;
            }
        }
    }
    hit
}

// ---------- shopping.py (Camofox Amazon resolver) ----------

const CAMOFOX_URL: &str = "http://127.0.0.1:9377";

pub fn amazon_asin(url: &str) -> String {
    let mut value = url.to_string();
    for _ in 0..2 {
        value = crate::vault::unescape_pub(&value);
    }
    let re = regex::Regex::new(r"(?i)/(?:dp|gp/product)/([A-Z0-9]{10})(?:[/?#]|$)").unwrap();
    re.captures(&value)
        .map(|c| c.get(1).unwrap().as_str().to_uppercase())
        .unwrap_or_default()
}

pub fn is_amazon_url(url: &str) -> bool {
    let host = crate::vault::host_of_pub(url).to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    host == "a.co" || host == "amzn.to" || host.starts_with("amazon.") || host.ends_with(".amazon.com")
}

pub fn amazon_search_url(query: &str) -> String {
    let q = regex::Regex::new(r"\s+").unwrap().replace_all(query.trim(), " ").to_string();
    format!("https://www.amazon.com/s?k={}", crate::vault::quote_plus_pub(&q))
}

fn camofox_request(method: &str, path: &str, payload: Option<&Value>, timeout: Duration) -> Value {
    let agent = http_agent(timeout);
    let mut req = match method {
        "POST" => agent.post(&format!("{CAMOFOX_URL}{path}")),
        "DELETE" => agent.delete(&format!("{CAMOFOX_URL}{path}")),
        _ => agent.get(&format!("{CAMOFOX_URL}{path}")),
    };
    if let Some(p) = payload {
        req = req.set("Content-Type", "application/json");
        let resp = req.send_string(&p.to_string());
        let Ok(resp) = resp else { return json!({}) };
        let mut text = String::new();
        use std::io::Read;
        let _ = resp.into_reader().read_to_string(&mut text);
        return serde_json::from_str(&text).unwrap_or(json!({}));
    }
    let Ok(resp) = req.call() else { return json!({}) };
    let mut text = String::new();
    use std::io::Read;
    let _ = resp.into_reader().read_to_string(&mut text);
    serde_json::from_str(&text).unwrap_or(json!({}))
}

/// Browser-automation user id for camofox. Configurable, so the public repo holds
/// no personal handle and an existing browser profile keeps working:
/// set TV_BROWSER_USER to the same value it used before.
fn camofox_user() -> String {
    std::env::var("TV_BROWSER_USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "admin".to_string())
}

fn camofox_snapshot(query: &str, timeout: u64) -> String {
    let target = amazon_search_url(query);
    let uid = camofox_user();
    let open = camofox_request(
        "POST",
        "/tabs",
        Some(&json!({"userId": &uid, "sessionKey": "taste-shopping"})),
        Duration::from_secs(45),
    );
    let tab = open.get("tabId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if tab.is_empty() {
        return String::new();
    }
    let mut snapshot = String::new();
    let _ = camofox_request(
        "POST",
        &format!("/tabs/{tab}/navigate"),
        Some(&json!({"userId": &uid, "url": target})),
        Duration::from_secs(45),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout);
    while std::time::Instant::now() < deadline {
        let data = camofox_request("GET", &format!("/tabs/{tab}/snapshot?userId={uid}"), None, Duration::from_secs(30));
        snapshot = data.get("snapshot").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if snapshot.contains("/dp/") && snapshot.contains("Results") {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let _ = camofox_request("DELETE", &format!("/tabs/{tab}?userId={uid}"), None, Duration::from_secs(20));
    snapshot
}

fn score_terms(value: &str) -> std::collections::HashSet<String> {
    let re = regex::Regex::new(r"[a-z0-9]+").unwrap();
    let stop = ["the", "and", "for", "with", "from", "this", "that", "inch"];
    re.find_iter(&value.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|w| !stop.contains(&w.as_str()))
        .collect()
}

fn shopping_score(query: &str, title: &str, rank: usize) -> f64 {
    let wanted = score_terms(query);
    let found = score_terms(title);
    let overlap = wanted.intersection(&found).count() as f64 / wanted.len().max(1) as f64;
    let mut score = overlap * 100.0 - rank as f64 * 0.1;
    let q = query.to_lowercase();
    let t = title.to_lowercase();
    for (phrase, bonus) in [("waveshare", 12.0), ("esp32-s3", 12.0), ("e-paper", 8.0), ("aiot", 8.0), ("200", 4.0)] {
        if q.contains(phrase) && t.contains(phrase) {
            score += bonus;
        }
    }
    if q.contains("black/white") && t.contains("black/white") {
        score += 6.0;
    }
    score
}

fn snapshot_results(snapshot: &str, query: &str) -> Vec<Value> {
    // split into "- listitem:" blocks (the lookahead-free port of
    // `- listitem:\n(.*?)(?=\n  - listitem:|\Z)`)
    let blocks: Vec<&str> = snapshot
        .split("\n  - listitem:")
        .map(|b| b.strip_prefix("- listitem:").unwrap_or(b).trim_start_matches('\n'))
        .collect();
    let re_asin = regex::Regex::new(r"(?i)/dp/([A-Z0-9]{10})").unwrap();
    let re_label = regex::Regex::new(r#"(?i)- (?:link|heading) "(?P<d>.*?)"|'(?P<s>.*?)'"#).unwrap();
    let mut results: Vec<Value> = Vec::new();
    for (rank, block) in blocks.iter().enumerate() {
        let block_text = crate::vault::unescape_pub(block);
        let Some(asin_cap) = re_asin.captures(&block_text) else { continue };
        let mut labels: Vec<String> = Vec::new();
        for cap in re_label.captures_iter(&block_text) {
            let raw = cap.name("d").or_else(|| cap.name("s")).map(|m| m.as_str()).unwrap_or("");
            let mut label = crate::vault::unescape_pub(raw).trim().to_string();
            if let Ok(re_spon) = regex::Regex::new(r"(?i)^Sponsored Ad\s*-\s*") {
                label = re_spon.replace_all(&label, "").trim().to_string();
            }
            let lower = label.to_lowercase();
            if !label.is_empty()
                && !labels.contains(&label)
                && lower != "results"
                && lower != "price, product page"
            {
                labels.push(label);
            }
        }
        if labels.is_empty() {
            continue;
        }
        let mut title = labels
            .iter()
            .max_by_key(|l| score_terms(l).len())
            .unwrap()
            .clone();
        let query_lower = query.to_lowercase();
        for brand in ["waveshare", "seeed", "adafruit", "sparkfun"] {
            if query_lower.contains(brand) && !title.to_lowercase().contains(brand) {
                let pat = format!(r"(?i)\b{}\b", regex::escape(brand));
                if let Ok(re) = regex::Regex::new(&pat) {
                    if re.is_match(&block_text) {
                        let mut c = brand.to_string();
                        c.push(' ');
                        c.push_str(&title);
                        title = c;
                        break;
                    }
                }
            }
        }
        let asin = asin_cap.get(1).unwrap().as_str().to_uppercase();
        results.push(json!({
            "url": format!("https://www.amazon.com/dp/{asin}"),
            "title": title,
            "score": shopping_score(query, &title, rank),
        }));
    }
    // dedupe by url keeping first, then sort by score desc
    let mut seen: Vec<String> = Vec::new();
    let mut deduped: Vec<Value> = Vec::new();
    for r in results {
        let u = r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if seen.contains(&u) {
            continue;
        }
        seen.push(u);
        deduped.push(r);
    }
    deduped.sort_by(|a, b| {
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    deduped
}

pub fn resolve_amazon_product(product_name: &str, timeout: u64) -> Value {
    let query = regex::Regex::new(r"\s+").unwrap().replace_all(product_name.trim(), " ").to_string();
    if query.is_empty() {
        return json!({});
    }
    let snapshot = camofox_snapshot(&query, timeout);
    let results = snapshot_results(&snapshot, &query);
    if let Some(first) = results.first() {
        let mut r = first.clone();
        r.as_object_mut().unwrap().insert("search_url".into(), json!(amazon_search_url(&query)));
        return r;
    }
    json!({
        "url": amazon_search_url(&query),
        "title": query,
        "search_url": amazon_search_url(&query),
        "is_search": true,
    })
}
