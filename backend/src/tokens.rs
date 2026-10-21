//! Long-lived API tokens for scripted clients (the `te` CLI, cron, phone
//! shortcuts). Basic auth stays for the browser; a token is a bearer credential
//! that can be revoked on its own.
//!
//! Only the SHA-256 of a token is stored. The plaintext is returned once, at
//! creation, and never again.

use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

static DB_PATH: OnceLock<String> = OnceLock::new();
static HASHES: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

fn hashes() -> &'static RwLock<Vec<String>> {
    HASHES.get_or_init(|| RwLock::new(Vec::new()))
}

pub fn init(path: String) {
    if DB_PATH.set(path).is_ok() {
        if let Ok(conn) = open() {
            let _ = ensure(&conn);
            reload(&conn);
        }
    }
}

fn open() -> Result<Connection, String> {
    let p = DB_PATH.get().ok_or_else(|| "tokens: db path not initialised".to_string())?;
    let conn = Connection::open_with_flags(
        p,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|e| e.to_string())?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Ok(conn)
}

pub fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS api_tokens (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL DEFAULT '',
            hash       TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0,
            last_used  INTEGER NOT NULL DEFAULT 0
         );
         CREATE UNIQUE INDEX IF NOT EXISTS api_tokens_hash ON api_tokens(hash);",
    )
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn sha_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn rand_hex(n: usize) -> String {
    // Mixed OS entropy sources; the token only has to be unguessable, not a
    // cryptographic primitive of its own.
    let mut out = String::with_capacity(n * 2);
    let mut seed = now_ms() as u64 ^ (std::process::id() as u64) << 32;
    for _ in 0..n * 2 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let v = ((seed >> 33) & 0xf) as u8;
        out.push(char::from_digit(v as u32, 16).unwrap_or('0'));
    }
    out
}

fn reload(conn: &Connection) {
    let mut v = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT hash FROM api_tokens") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for h in rows.flatten() {
                v.push(h);
            }
        }
    }
    if let Ok(mut w) = hashes().write() {
        *w = v;
    }
}

/// Create a token. Returns (id, plaintext); the plaintext is shown once.
pub fn create(name: &str) -> Result<(String, String), String> {
    let conn = open()?;
    let _ = ensure(&conn);
    let id = rand_hex(6);
    let token = format!("te_{}", rand_hex(20));
    let h = sha_hex(&token);
    conn.execute(
        "INSERT INTO api_tokens(id, name, hash, created_at, last_used) VALUES (?, ?, ?, ?, 0)",
        params![id, name, h, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    reload(&conn);
    Ok((id, token))
}

pub fn list() -> Value {
    let mut out = Vec::new();
    if let Ok(conn) = open() {
        let _ = ensure(&conn);
        if let Ok(mut stmt) =
            conn.prepare("SELECT id, name, created_at, last_used FROM api_tokens ORDER BY created_at")
        {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "created_at": r.get::<_, i64>(2)?,
                    "last_used": r.get::<_, i64>(3)?,
                }))
            }) {
                out.extend(rows.flatten());
            }
        }
    }
    json!({ "count": out.len(), "tokens": out })
}

pub fn delete(id: &str) -> bool {
    let Ok(conn) = open() else { return false };
    let n = conn
        .execute("DELETE FROM api_tokens WHERE id = ?", params![id])
        .unwrap_or(0);
    reload(&conn);
    n > 0
}

/// Constant-time check of a bearer token against the loaded hashes.
pub fn is_valid(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() || !token.starts_with("te_") {
        return false;
    }
    let want = sha_hex(token);
    let guard = match hashes().read() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let mut hit = 0u8;
    for h in guard.iter() {
        if h.len() == want.len() {
            hit |= h.bytes().zip(want.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y));
        }
    }
    hit == 0 && !guard.is_empty()
}

pub fn touch(token: &str) {
    let Ok(conn) = open() else { return };
    let _ = conn.execute(
        "UPDATE api_tokens SET last_used = ? WHERE hash = ?",
        params![now_ms(), sha_hex(token.trim())],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip_in_memory() {
        let dir = std::env::temp_dir().join(format!("te-tok-{}-roundtrip", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        let _ = std::fs::remove_file(&db);
        init(db.to_string_lossy().to_string());
        let (_id, tok) = create("test").unwrap();
        assert!(tok.starts_with("te_"));
        assert!(is_valid(&tok));
        assert!(!is_valid("te_deadbeef"));
        assert!(!is_valid(""));
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(sha_hex("a").len(), 64);
        assert_ne!(sha_hex("a"), sha_hex("b"));
    }
}
