//! Source registry: one declarative table for every platform the vault knows.
//!
//! Port of Pond's `docs/supported-sources.md` idea (id + hosts + URL patterns +
//! sync strategy per source) so adding a platform is one row plus one extractor
//! instead of another branch inside ingest.
//!
//! PATTERN RULE: the `regex` crate has NO look-around. Every pattern here is
//! plain and must stay compilable by `regex::Regex::new` (a lookahead panics the
//! compiling worker; see the ingest incident of 2026-10-04).

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub struct SourceDef {
    pub id: &'static str,
    pub label: &'static str,
    pub hosts: &'static [&'static str],
    /// URL patterns that identify a single saved object on this platform.
    pub patterns: &'static [&'static str],
    /// How a saved list is walked: `cursor`, `scroll`, `full`, or `-` (capture only).
    pub sync: &'static str,
}

pub const SOURCES: &[SourceDef] = &[
    SourceDef {
        id: "x",
        label: "X",
        hosts: &["x.com", "twitter.com", "mobile.twitter.com"],
        patterns: &[
            r"^https?://(?:www\.)?(?:x|twitter)\.com/[^/]+/status/\d+",
            r"^https?://(?:mobile\.)?(?:x|twitter)\.com/i/web/status/\d+",
        ],
        sync: "cursor",
    },
    SourceDef {
        id: "instagram",
        label: "Instagram",
        hosts: &["instagram.com"],
        patterns: &[
            r"^https?://(?:www\.)?instagram\.com/(?:p|reels?|tv)/[A-Za-z0-9_-]+",
            r"^https?://(?:www\.)?instagram\.com/stories/[A-Za-z0-9_.]+",
        ],
        sync: "cursor",
    },
    SourceDef {
        id: "pinterest",
        label: "Pinterest",
        hosts: &["pinterest.com", "pin.it"],
        patterns: &[
            r"^https?://(?:[a-z]+\.)?pinterest\.[a-z.]+/pin/\d+",
            r"^https?://pin\.it/[A-Za-z0-9]+",
        ],
        sync: "scroll",
    },
    SourceDef {
        id: "reddit",
        label: "Reddit",
        hosts: &["reddit.com", "redd.it", "old.reddit.com"],
        patterns: &[
            r"^https?://(?:[a-z]+\.)?reddit\.com/r/[^/]+/comments/[a-z0-9]+",
            r"^https?://(?:[a-z]+\.)?reddit\.com/comments/[a-z0-9]+",
            r"^https?://redd\.it/[a-z0-9]+",
        ],
        sync: "cursor",
    },
    SourceDef {
        id: "youtube",
        label: "YouTube",
        hosts: &["youtube.com", "youtu.be", "m.youtube.com"],
        patterns: &[
            r"^https?://(?:www\.|m\.)?youtube\.com/watch\?[^#]*v=[A-Za-z0-9_-]+",
            r"^https?://(?:www\.)?youtube\.com/shorts/[A-Za-z0-9_-]+",
            r"^https?://youtu\.be/[A-Za-z0-9_-]+",
        ],
        sync: "scroll",
    },
    SourceDef {
        id: "tiktok",
        label: "TikTok",
        hosts: &["tiktok.com", "vm.tiktok.com", "vt.tiktok.com"],
        patterns: &[
            r"^https?://(?:[a-z]+\.)?tiktok\.com/[^/]+/video/\d+",
            r"^https?://(?:[a-z]+\.)?tiktok\.com/video/\d+",
            r"^https?://(?:vm|vt)\.tiktok\.com/[A-Za-z0-9]+",
        ],
        sync: "scroll",
    },
    SourceDef {
        id: "threads",
        label: "Threads",
        hosts: &["threads.net", "threads.com"],
        patterns: &[
            r"^https?://(?:www\.)?threads\.(?:net|com)/@[^/]+/post/[A-Za-z0-9_-]+",
            r"^https?://(?:www\.)?threads\.(?:net|com)/t/[A-Za-z0-9_-]+",
        ],
        sync: "scroll",
    },
    SourceDef {
        id: "bluesky",
        label: "Bluesky",
        hosts: &["bsky.app"],
        patterns: &[r"^https?://bsky\.app/profile/[^/]+/post/[A-Za-z0-9]+"],
        sync: "cursor",
    },
    SourceDef {
        id: "tumblr",
        label: "Tumblr",
        hosts: &["tumblr.com"],
        patterns: &[r"^https?://(?:[a-z0-9-]+\.)?tumblr\.com/post/\d+"],
        sync: "cursor",
    },
    SourceDef {
        id: "arena",
        label: "Are.na",
        hosts: &["are.na"],
        patterns: &[
            r"^https?://(?:www\.)?are\.na/block/\d+",
            r"^https?://(?:www\.)?are\.na/[^/]+/[^/]+-\d+",
        ],
        sync: "scroll",
    },
    SourceDef {
        id: "cosmos",
        label: "Cosmos",
        hosts: &["cosmos.so"],
        patterns: &[r"^https?://cosmos\.so/e/[A-Za-z0-9_-]+"],
        sync: "cursor",
    },
    SourceDef {
        id: "steam",
        label: "Steam",
        hosts: &["store.steampowered.com", "steamcommunity.com"],
        patterns: &[
            r"^https?://store\.steampowered\.com/app/\d+",
            r"^https?://steamcommunity\.com/app/\d+",
        ],
        sync: "-",
    },
    SourceDef {
        id: "spotify",
        label: "Spotify",
        hosts: &["open.spotify.com"],
        patterns: &[r"^https?://open\.spotify\.com/(?:track|album|playlist|episode)/[A-Za-z0-9]+"],
        sync: "-",
    },
];

fn compiled() -> &'static HashMap<&'static str, Vec<Regex>> {
    static COMPILED: OnceLock<HashMap<&'static str, Vec<Regex>>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        let mut m = HashMap::new();
        for s in SOURCES {
            let mut v = Vec::new();
            for p in s.patterns {
                match Regex::new(p) {
                    Ok(r) => v.push(r),
                    Err(e) => eprintln!("taste-engine-rs: source pattern {} {p}: {e}", s.id),
                }
            }
            m.insert(s.id, v);
        }
        m
    })
}

/// Best-effort source id for a URL. Host match first, then pattern match, so a
/// `pin.it` shortener resolves even when the path is opaque.
pub fn source_of(url: &str) -> Option<&'static str> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let host = crate::vault::host_of_pub(trimmed);
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let comp = compiled();
    for s in SOURCES {
        if s.hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))) {
            if s.patterns.is_empty() {
                return Some(s.id);
            }
            if comp.get(s.id).map(|v| v.iter().any(|r| r.is_match(trimmed))).unwrap_or(false) {
                return Some(s.id);
            }
        }
    }
    for s in SOURCES {
        if comp.get(s.id).map(|v| v.iter().any(|r| r.is_match(trimmed))).unwrap_or(false) {
            return Some(s.id);
        }
    }
    None
}

pub fn label_of(id: &str) -> &'static str {
    SOURCES.iter().find(|s| s.id == id).map(|s| s.label).unwrap_or("web")
}

pub fn is_social(id: &str) -> bool {
    !matches!(id, "steam" | "spotify")
}

pub fn json_table() -> Value {
    let rows: Vec<Value> = SOURCES
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "label": s.label,
                "hosts": s.hosts,
                "patterns": s.patterns,
                "sync": s.sync,
                "social": is_social(s.id),
            })
        })
        .collect();
    json!({ "count": rows.len(), "sources": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        // The regex crate panics the compiling worker on look-around, so a broken
        // pattern must fail loudly here instead of at request time.
        assert_eq!(compiled().len(), SOURCES.len());
        for (id, v) in compiled() {
            assert_eq!(v.len(), SOURCES.iter().find(|s| s.id == *id).unwrap().patterns.len(), "{id}");
        }
    }

    #[test]
    fn resolves_known_urls() {
        assert_eq!(source_of("https://x.com/raphaelsalaja/status/2110722616226914684"), Some("x"));
        assert_eq!(source_of("https://www.instagram.com/reel/Cx1abcDEF_/?igsh=1"), Some("instagram"));
        assert_eq!(source_of("https://pin.it/abc123"), Some("pinterest"));
        assert_eq!(source_of("https://youtu.be/dQw4w9WgXcQ"), Some("youtube"));
        assert_eq!(source_of("https://are.na/block/12345"), Some("arena"));
        assert_eq!(source_of("https://example.com/post/1"), None);
    }

    #[test]
    fn labels_never_empty() {
        assert_eq!(label_of("x"), "X");
        assert_eq!(label_of("nope"), "web");
    }
}
