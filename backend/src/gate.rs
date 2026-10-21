//! Per-source admission gate with cooldowns that survive a restart.
//!
//! Why: the scraper retries with in-memory backoff, so a restart (or a worker
//! respawn) forgets a 429 and hits the platform again immediately. Pond learned
//! the same lesson ("better to sync slowly than to risk a user's account"), so
//! cooldowns, breaker trips and request budgets live in SQLite, not memory.
//!
//! Vocabulary used by the whole module: ADMIT before a request, REPORT after it.
//! A denied request carries the reason and the epoch-millis it may retry at.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};

use crate::sources;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const BREAKER_THRESHOLD: i64 = 3;
const BREAKER_TRIP_MS: i64 = 900_000;
const BACKOFF_CAP_MS: i64 = 6 * HOUR_MS;
const CHALLENGE_COOLDOWN_MS: i64 = 1_800_000;

static DB_PATH: OnceLock<String> = OnceLock::new();

/// Call once at boot, before any worker thread starts.
pub fn init(path: String) {
    if DB_PATH.set(path).is_ok() {
        if let Ok(conn) = open() {
            let _ = ensure(&conn);
        }
    }
}

fn open() -> Result<Connection, String> {
    let p = DB_PATH.get().ok_or_else(|| "gate: db path not initialised".to_string())?;
    // CREATE: on a fresh install the gate tables are the first thing written.
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
        "CREATE TABLE IF NOT EXISTS source_gates (
            source     TEXT PRIMARY KEY,
            state      TEXT NOT NULL DEFAULT 'closed',
            reason     TEXT NOT NULL DEFAULT '',
            retry_at   INTEGER NOT NULL DEFAULT 0,
            failures   INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS source_ledger (
            source TEXT NOT NULL,
            ts     INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS source_ledger_ts ON source_ledger(source, ts);",
    )
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn per_hour() -> i64 {
    env_i64("TV_GATE_PER_HOUR", 120)
}

pub fn per_day() -> i64 {
    env_i64("TV_GATE_PER_DAY", 1200)
}

/// (state, reason, retry_at, failures) for one source, creating the row on demand.
fn row(conn: &Connection, source: &str) -> (String, String, i64, i64) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO source_gates(source, state, reason, retry_at, failures, updated_at)
         VALUES (?, 'closed', '', 0, 0, ?)",
        params![source, now_ms()],
    );
    conn.query_row(
        "SELECT state, reason, retry_at, failures FROM source_gates WHERE source = ?",
        params![source],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
    )
    .unwrap_or_else(|_| ("closed".into(), String::new(), 0, 0))
}

/// ADMIT one request to `source`. `Err((reason, retry_at_ms))` means do not send.
pub fn admit(source: &str) -> Result<(), (String, i64)> {
    let conn = open().map_err(|e| (e, now_ms()))?;
    let t = now_ms();
    let (state, reason, retry_at, _) = row(&conn, source);
    if state == "paused" {
        return Err(("source_paused".into(), i64::MAX));
    }
    if retry_at > t {
        return Err((if reason.is_empty() { "cooldown".into() } else { reason }, retry_at));
    }
    // A platform home feed is not a free API: cap requests per window.
    let hour_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM source_ledger WHERE source = ? AND ts > ?",
            params![source, t - HOUR_MS],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if hour_count >= per_hour() {
        let oldest: i64 = conn
            .query_row(
                "SELECT MIN(ts) FROM source_ledger WHERE source = ? AND ts > ?",
                params![source, t - HOUR_MS],
                |r| r.get(0),
            )
            .unwrap_or(t);
        return Err(("source_budget".into(), oldest + HOUR_MS));
    }
    let day_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM source_ledger WHERE source = ? AND ts > ?",
            params![source, t - DAY_MS],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if day_count >= per_day() {
        let oldest: i64 = conn
            .query_row(
                "SELECT MIN(ts) FROM source_ledger WHERE source = ? AND ts > ?",
                params![source, t - DAY_MS],
                |r| r.get(0),
            )
            .unwrap_or(t);
        return Err(("source_budget".into(), oldest + DAY_MS));
    }
    Ok(())
}

/// REPORT the outcome of one request. 2xx closes the gate and spends one permit.
/// 429 cools down with exponential backoff, 403 means a challenge, 5xx cools
/// down briefly, and three failures in a row trip the breaker.
pub fn report(source: &str, status: u16, retry_after_s: Option<i64>) {
    let Ok(conn) = open() else { return };
    let t = now_ms();
    let (_, _, _, failures) = row(&conn, source);
    if (200..400).contains(&status) {
        let _ = conn.execute(
            "UPDATE source_gates SET state='closed', reason='', retry_at=0, failures=0, updated_at=? WHERE source=?",
            params![t, source],
        );
        let _ = conn.execute("INSERT INTO source_ledger(source, ts) VALUES (?, ?)", params![source, t]);
        let _ = conn.execute("DELETE FROM source_ledger WHERE ts < ?", params![t - 2 * DAY_MS]);
        return;
    }
    let fails = failures + 1;
    let (reason, cool): (&str, i64) = match status {
        401 | 403 => ("challenge", CHALLENGE_COOLDOWN_MS),
        429 => {
            let base = env_i64("TV_GATE_BACKOFF_BASE", 300_000);
            let exp = base.saturating_mul(1i64 << fails.min(6) as u32);
            let server = retry_after_s.unwrap_or(0) * 1000;
            ("rate_limited", server.max(exp).min(BACKOFF_CAP_MS))
        }
        500..=599 => ("upstream_error", 120_000),
        _ => ("blocked", 300_000),
    };
    let mut until = t + cool;
    if fails >= BREAKER_THRESHOLD {
        until = until.max(t + BREAKER_TRIP_MS);
    }
    let _ = conn.execute(
        "INSERT INTO source_gates(source, state, reason, retry_at, failures, updated_at)
         VALUES (?, 'open', ?, ?, ?, ?)
         ON CONFLICT(source) DO UPDATE SET state='open', reason=excluded.reason,
            retry_at=excluded.retry_at, failures=excluded.failures, updated_at=excluded.updated_at",
        params![source, reason, until, fails, t],
    );
}

pub fn pause(source: &str) {
    if let Ok(conn) = open() {
        let _ = conn.execute(
            "INSERT INTO source_gates(source, state, reason, retry_at, failures, updated_at)
             VALUES (?, 'paused', 'manual', 0, 0, ?)
             ON CONFLICT(source) DO UPDATE SET state='paused', reason='manual', retry_at=0, updated_at=excluded.updated_at",
            params![source, now_ms()],
        );
    }
}

pub fn resume(source: &str) {
    if let Ok(conn) = open() {
        let _ = conn.execute(
            "UPDATE source_gates SET state='closed', reason='', retry_at=0, failures=0, updated_at=? WHERE source=?",
            params![now_ms(), source],
        );
    }
}

/// Gate snapshot for the UI and for the scraper's pre-flight check.
pub fn snapshot() -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let t = now_ms();
    if let Ok(conn) = open() {
        let _ = ensure(&conn);
        for s in sources::SOURCES {
            let (state, reason, retry_at, failures) = row(&conn, s.id);
            let hour: i64 = conn
                .query_row("SELECT COUNT(*) FROM source_ledger WHERE source=? AND ts>?", params![s.id, t - HOUR_MS], |r| r.get(0))
                .unwrap_or(0);
            let day: i64 = conn
                .query_row("SELECT COUNT(*) FROM source_ledger WHERE source=? AND ts>?", params![s.id, t - DAY_MS], |r| r.get(0))
                .unwrap_or(0);
            rows.push(json!({
                "source": s.id,
                "label": s.label,
                "state": state,
                "reason": reason,
                "retry_at": if retry_at > t { retry_at } else { 0 },
                "retry_in_ms": (retry_at - t).max(0),
                "failures": failures,
                "requests_last_hour": hour,
                "requests_last_day": day,
                "budget_per_hour": per_hour(),
                "budget_per_day": per_day(),
            }));
        }
    }
    json!({ "now": t, "sources": rows })
}

/// Download with the gate applied. Every byte the server pulls from a social
/// host goes through here.
pub fn fetch(url: &str, timeout_s: u64) -> Result<(Vec<u8>, String), String> {
    let source = sources::source_of(url);
    if let Some(src) = source {
        if sources::is_social(src) {
            if let Err((reason, retry_at)) = admit(src) {
                return Err(format!("gate:{reason}:{retry_at}"));
            }
        }
    }
    let agent = crate::mediaio::http_agent(std::time::Duration::from_secs(timeout_s));
    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            if let Some(src) = source {
                let retry = r.header("retry-after").and_then(|v| v.parse::<i64>().ok());
                report(src, code, retry);
            }
            return Err(format!("download: http {code}"));
        }
        Err(e) => {
            if let Some(src) = source {
                report(src, 599, None);
            }
            return Err(format!("download: {e}"));
        }
    };
    let status = resp.status();
    let ctype = resp
        .header("content-type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if let Some(src) = source {
        let retry = resp.header("retry-after").and_then(|v| v.parse::<i64>().ok());
        report(src, status, retry);
    }
    let max = crate::mediaio::media_keep_bytes();
    let mut data: Vec<u8> = Vec::new();
    use std::io::Read;
    let mut reader = resp.into_reader();
    let mut chunk = [0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
        if data.len() as i64 > max {
            return Err(format!("file too large: {url}"));
        }
    }
    Ok((data, ctype))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(tag: &str) -> Connection {
        let dir = std::env::temp_dir().join(format!("te-gate-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        ensure(&conn).unwrap();
        conn
    }

    #[test]
    fn cooldown_persists_in_the_table() {
        let conn = tmp_db("cooldown");
        let t = now_ms();
        conn.execute(
            "INSERT INTO source_gates(source,state,reason,retry_at,failures,updated_at)
             VALUES ('x','open','rate_limited',?,1,?)",
            params![t + 600_000, t],
        )
        .unwrap();
        let (state, reason, retry_at, _) = row(&conn, "x");
        assert_eq!(state, "open");
        assert_eq!(reason, "rate_limited");
        assert!(retry_at > t);
    }

    #[test]
    fn manual_pause_survives() {
        let conn = tmp_db("pause");
        conn.execute(
            "INSERT INTO source_gates(source,state,reason,retry_at,failures,updated_at)
             VALUES ('pinterest','paused','manual',0,0,0)",
            params![],
        )
        .unwrap();
        let (state, _, _, _) = row(&conn, "pinterest");
        assert_eq!(state, "paused");
    }
}
