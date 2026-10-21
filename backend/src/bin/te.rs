//! te — command line client for a running Taste Engine server.
//!
//! Auth: TE_TOKEN (bearer, created with `te token create`) or TE_BASIC
//! `user:pass`. Base URL: TE_URL, default http://127.0.0.1:8770.
//!
//!   te ls [--limit 20] [--kind image] [--source x]
//!   te search scenic --limit 10
//!   te save https://x.com/user/status/1
//!   te sources
//!   te gates
//!   te origin <media_id>
//!   te token create [name] | te token list | te token rm <id>

use std::env;
use std::process::exit;

fn base() -> String {
    env::var("TE_URL").unwrap_or_else(|_| "http://127.0.0.1:8770".to_string())
}

fn auth_header() -> Option<String> {
    if let Ok(tok) = env::var("TE_TOKEN") {
        if !tok.trim().is_empty() {
            return Some(format!("Bearer {}", tok.trim()));
        }
    }
    // TE_BASIC, else the server's own TV_USER/TV_PASS. The container carries those,
    // so `docker exec taste-engine /app/te gates` works with no extra flags.
    let pair = env::var("TE_BASIC").ok().or_else(|| match (env::var("TV_USER"), env::var("TV_PASS")) {
        (Ok(u), Ok(p)) if !u.is_empty() && !p.is_empty() => Some(format!("{u}:{p}")),
        _ => None,
    });
    if let Some(b) = pair {
        if let Some((u, p)) = b.split_once(':') {
            use base64::Engine;
            let raw = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"));
            return Some(format!("Basic {raw}"));
        }
    }
    None
}

fn req(method: &str, path: &str, body: Option<String>) -> (u16, String) {
    let url = format!("{}{}", base(), path);
    let mut r = match method {
        "GET" => ureq::get(&url),
        "POST" => ureq::post(&url),
        "DELETE" => ureq::delete(&url),
        _ => ureq::get(&url),
    };
    if let Some(a) = auth_header() {
        r = r.set("Authorization", &a);
    }
    let result = if let Some(b) = body {
        r.set("Content-Type", "application/json").send_string(&b)
    } else {
        r.call()
    };
    match result {
        Ok(resp) => {
            let code = resp.status();
            (code, resp.into_string().unwrap_or_default())
        }
        Err(ureq::Error::Status(code, resp)) => (code, resp.into_string().unwrap_or_default()),
        Err(e) => (0, format!("{{\"detail\":\"{e}\"}}")),
    }
}

fn pretty(raw: &str) {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(_) => println!("{raw}"),
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a.starts_with("--") {
            skip = true;
            continue;
        }
        out.push(a.clone());
    }
    out
}

fn main() {
    // Rust ignores SIGPIPE, so a closed pipe makes println! panic. Restore the
    // Unix default so `te ls | head` exits quietly like every other CLI.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: te <ls|search|save|sources|gates|origin|token> [args]");
        exit(2);
    }
    let pos = positional(&args);
    let cmd = pos.first().map(|s| s.as_str()).unwrap_or("");
    let limit = flag(&args, "--limit").unwrap_or_else(|| "20".to_string());
    match cmd {
        "ls" => {
            let mut q = format!("/api/items?limit={limit}");
            if let Some(k) = flag(&args, "--kind") {
                q.push_str(&format!("&kind={k}"));
            }
            if let Some(s) = flag(&args, "--source") {
                q.push_str(&format!("&source={s}"));
            }
            let (code, body) = req("GET", &q, None);
            if code != 200 {
                pretty(&body);
                exit(1);
            }
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
            for it in &items {
                println!(
                    "{}  {:<6} {:<12} {}",
                    it.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                    it.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                    it.get("source").and_then(|x| x.as_str()).unwrap_or(""),
                    it.get("title").and_then(|x| x.as_str()).unwrap_or("").chars().take(70).collect::<String>(),
                );
            }
            eprintln!("{} shown", items.len());
        }
        "search" => {
            let q = pos.get(1).cloned().unwrap_or_default();
            if q.is_empty() {
                eprintln!("usage: te search <query>");
                exit(2);
            }
            let enc = urlencode(&q);
            let (code, body) = req("GET", &format!("/api/search?q={enc}&limit={limit}"), None);
            if code != 200 {
                pretty(&body);
                exit(1);
            }
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            for it in v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default() {
                println!(
                    "{:.3}  {}  {}",
                    it.get("similarity").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    it.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                    it.get("title").and_then(|x| x.as_str()).unwrap_or("").chars().take(70).collect::<String>(),
                );
            }
        }
        "save" => {
            let url = pos.get(1).cloned().unwrap_or_default();
            if url.is_empty() {
                eprintln!("usage: te save <url>");
                exit(2);
            }
            let body = serde_json::json!({ "url": url }).to_string();
            let (code, out) = req("POST", "/api/items", Some(body));
            pretty(&out);
            if code >= 400 {
                exit(1);
            }
        }
        "sources" => {
            let (code, body) = req("GET", "/api/sources", None);
            if code != 200 {
                pretty(&body);
                exit(1);
            }
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            for s in v.get("sources").and_then(|a| a.as_array()).cloned().unwrap_or_default() {
                println!(
                    "{:<10} {:<12} sync={:<7} hosts={}",
                    s.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                    s.get("label").and_then(|x| x.as_str()).unwrap_or(""),
                    s.get("sync").and_then(|x| x.as_str()).unwrap_or(""),
                    s.get("hosts").and_then(|h| h.as_array()).map(|h| h.len()).unwrap_or(0),
                );
            }
        }
        "gates" => {
            let (code, body) = req("GET", "/api/sources/gates", None);
            if code != 200 {
                pretty(&body);
                exit(1);
            }
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            for s in v.get("sources").and_then(|a| a.as_array()).cloned().unwrap_or_default() {
                let state = s.get("state").and_then(|x| x.as_str()).unwrap_or("closed");
                let retry = s.get("retry_in_ms").and_then(|x| x.as_i64()).unwrap_or(0);
                println!(
                    "{:<10} {:<14} {:>4}s  {}/{}",
                    s.get("source").and_then(|x| x.as_str()).unwrap_or(""),
                    state,
                    retry / 1000,
                    s.get("requests_last_hour").and_then(|x| x.as_i64()).unwrap_or(0),
                    s.get("budget_per_hour").and_then(|x| x.as_i64()).unwrap_or(0),
                );
            }
        }
        "origin" => {
            let id = pos.get(1).cloned().unwrap_or_default();
            if id.is_empty() {
                eprintln!("usage: te origin <media_id>");
                exit(2);
            }
            let (code, body) = req("GET", &format!("/api/media/{id}/origin"), None);
            pretty(&body);
            if code >= 400 {
                exit(1);
            }
        }
        "token" => {
            let sub = pos.get(1).map(|s| s.as_str()).unwrap_or("list");
            match sub {
                "create" => {
                    let name = pos.get(2).cloned().unwrap_or_else(|| "cli".to_string());
                    let body = serde_json::json!({ "name": name }).to_string();
                    let (code, out) = req("POST", "/api/tokens", Some(body));
                    pretty(&out);
                    if code >= 400 {
                        eprintln!("token create needs TE_BASIC=<user>:<pass> (Basic auth)");
                        exit(1);
                    }
                }
                "rm" | "delete" => {
                    let id = pos.get(2).cloned().unwrap_or_default();
                    let (code, out) = req("DELETE", &format!("/api/tokens/{id}"), None);
                    pretty(&out);
                    if code >= 400 {
                        exit(1);
                    }
                }
                _ => {
                    let (code, out) = req("GET", "/api/tokens", None);
                    pretty(&out);
                    if code >= 400 {
                        exit(1);
                    }
                }
            }
        }
        other => {
            eprintln!("unknown command: {other}");
            exit(2);
        }
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
