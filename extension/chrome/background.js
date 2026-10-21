// Taste Engine Saver — background page (Firefox/Zen variant).
"use strict";

const DEFAULT_SERVERS = ["http://127.0.0.1:8770"];

function getConfig() {
  return chrome.storage.local.get({
    server: "", user: "", pass: "", tags: [],
    stopRatio: 0.20,
    hotkeyImage: "alt+shift+s",
    hotkeyLink: "alt+shift+l",
    autoSync: { enabled: false, intervalHours: 6, lastSync: "", lastResult: "" },
    sources: {
      twitter: { enabled: true, url: "https://x.com/i/history" },
      reddit: { enabled: false, url: "" },
      instagram: { enabled: false, url: "" },
      pinterest: { enabled: false, url: "" },
    },
  });
}

function serverUrl(cfg) {
  let s = (cfg.server || "").trim().replace(/\/+$/, "");
  if (s) return s;
  for (const d of DEFAULT_SERVERS) {
    s = d;
    break;
  }
  return s;
}

function basicAuth(cfg) {
  if (!cfg.user) return "";
  return "Basic " + btoa(cfg.user + ":" + cfg.pass);
}

async function ensureOrigin(url) {
  const origin = new URL(url).origin + "/*";
  const granted = await chrome.permissions.contains({ origins: [origin] });
  if (granted) return true;
  return chrome.permissions.request({ origins: [origin] });
}

async function notify(title, message) {
  try {
    await chrome.notifications.create({
      type: "basic",
      iconUrl: "icons/icon-128.png",
      title: title,
      message: message,
    });
  } catch (e) {}
}

// Show or update the mymind-style main-view panel in a tab.
function sendTabState(tabId, state) {
  if (!tabId) return false;
  try {
    chrome.tabs.sendMessage(tabId, Object.assign({ type: "tv-save-state" }, state));
    return true;
  } catch (e) {
    return false;
  }
}

// Save any payload to the vault. Returns {ok, id?, error?}.
async function savePayload(payload, cfg) {
  const base = serverUrl(cfg);
  if (!base) return { ok: false, error: "set server URL in extension options" };
  if (!(await ensureOrigin(base + "/"))) {
    return { ok: false, error: "host permission denied" };
  }
  try {
    const res = await fetch(base + "/api/items", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(basicAuth(cfg) ? { Authorization: basicAuth(cfg) } : {}),
      },
      body: JSON.stringify(payload),
    });
    if (res.status === 401) return { ok: false, error: "wrong credentials — update extension options" };
    if (!res.ok) {
      let detail = res.statusText;
      try { detail = (await res.json()).detail || detail; } catch (e) {}
      return { ok: false, error: "HTTP " + res.status + " " + detail };
    }
    const data = await res.json();
    return { ok: true, id: data.id };
  } catch (e) {
    return { ok: false, error: "cannot reach server: " + e.message };
  }
}

async function saveWith(payload, label) {
  const cfg = await getConfig();
  const r = await savePayload(payload, cfg);
  return r;
}

// Save the page shown in `tab`. The toolbar button (one click) and the
// tv-save-tab message both come through here. On X status pages the visible
// thread texts ride along so the vault can harvest follow-up links.
async function saveTabPage(tab, tags) {
  const tabId = (tab && tab.id) || null;
  const url = (tab && tab.url) || "";
  if (!url) {
    if (tabId) sendTabState(tabId, { phase: "error", error: "nothing to save" });
    return { ok: false, error: "nothing to save" };
  }
  let thread = "";
  try {
    if (tab.id && /(^https?:\/\/(www\.)?(x|twitter)\.com\/)/i.test(url)
        && !/(i\/history|home|explore|notifications|messages)/i.test(url)) {
      const results = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: () => {
          const nodes = Array.from(document.querySelectorAll('[data-testid="tweetText"]'));
          const seen = new Set();
          return nodes
            .map((n) => (n.innerText || "").replace(/\s+/g, " ").trim())
            .filter((t) => t && !seen.has(t) && seen.add(t))
            .slice(0, 30);
        },
      });
      if (results && results[0] && Array.isArray(results[0].result)) {
        thread = results[0].result.join("\n\n");
      }
    }
  } catch (e) { /* optional capture; never block the save */ }
  const payload = {
    url,
    title: (tab && tab.title) || "",
    kind: "link",
    tags: tags || [],
    thread,
  };
  if (tabId) sendTabState(tabId, { phase: "saving", label: "Page" });
  const r = await saveWith(payload, "Page");
  if (tabId) {
    if (r.ok) sendTabState(tabId, { phase: "saved", id: r.id || "", label: "Page" });
    else sendTabState(tabId, { phase: "error", error: r.error || "The item was not saved" });
  }
  return r;
}

// The toolbar button is a one-click save: no default_popup in the manifest,
// so action.onClicked fires and saves the page in the active tab directly.
chrome.action.onClicked.addListener(async (tab) => {
  if (!tab || !/^https?:/i.test(tab.url || "")) {
    notify("Taste Engine", "This page cannot be saved");
    return;
  }
  try {
    const cfg = await getConfig();
    await saveTabPage(tab, cfg.tags || []);
  } catch (e) {
    notify("Taste Engine", "Could not save: " + ((e && e.message) || e));
  }
});

// Flat save menu: every item is one click — no hover submenu. "page" matches
// only when the click is NOT on a link/image/selection, so a right-click on a
// thumbnail (link + image) shows the link and image items, not the page item.
let menusBuilt = false;

function setupMenus() {
  if (menusBuilt) return;
  menusBuilt = true;
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "tv-page",
      title: "Save this page to Taste Engine",
      contexts: ["page"],
    });
    chrome.contextMenus.create({
      id: "tv-link",
      title: "Save this link to Taste Engine",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: "tv-image",
      title: "Save this image to Taste Engine",
      contexts: ["image"],
    });
    chrome.contextMenus.create({
      id: "tv-selection",
      title: "Save this selection to Taste Engine",
      contexts: ["selection"],
    });
    // Toolbar-icon menu (right-click the icon): keeps the actions the old
    // popup carried — open the vault and settings. Left-click = save page.
    chrome.contextMenus.create({
      id: "tv-action-open",
      title: "Open Taste Engine",
      contexts: ["action"],
    });
    chrome.contextMenus.create({
      id: "tv-action-settings",
      title: "Taste Engine settings",
      contexts: ["action"],
    });
  });
}

// Built at every background start: install, update, and temporary-add-on
// reload all restart the background context.
setupMenus();

// Fetch image bytes inside the TAB (page session/cookies). Returns a data
// URL or "". Used by the context-menu image save for private sources.
async function fetchImageBytesInTab(tabId, src) {
  if (!tabId || !src || !/^https?:/i.test(src)) return "";
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId },
      func: async (url) => {
        try {
          const res = await fetch(url, { credentials: "include", mode: "cors", redirect: "follow" });
          if (!res.ok) return "";
          const blob = await res.blob();
          if (!blob || blob.size < 200) return "";
          return await new Promise((resolve) => {
            const fr = new FileReader();
            fr.onload = () => resolve(String(fr.result || ""));
            fr.onerror = () => resolve("");
            fr.readAsDataURL(blob);
          });
        } catch (e) { return ""; }
      },
      args: [src],
    });
    const out = results && results[0] && results[0].result;
    return typeof out === "string" && out.startsWith("data:") ? out : "";
  } catch (e) {
    return "";
  }
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const cfg = await getConfig();
  const payload = { tags: cfg.tags || [] };
  let label = "";
  switch (info.menuItemId) {
    case "tv-page":
      payload.url = info.pageUrl || (tab && tab.url) || "";
      payload.title = (tab && tab.title) || "";
      payload.kind = "link";
      label = "Page";
      break;
    case "tv-link":
      payload.url = info.linkUrl || "";
      payload.title = (info.selectionText || "").slice(0, 200);
      payload.kind = "link";
      label = "Link";
      break;
    case "tv-image": {
      const src = info.srcUrl || "";
      const imgTitle = (info.altText || "saved image").slice(0, 200);
      // Try to fetch the bytes FROM THE PAGE (own session) first —
      // private/self-hosted sources (Immich) bounce server-side downloads.
      const dataUrl = await fetchImageBytesInTab(tab && tab.id, src);
      if (dataUrl) {
        await uploadImageStaged(cfg, tab && tab.id, {
          url: src, title: imgTitle, tags: cfg.tags || [], dataUrl,
        });
        return;
      }
      payload.url = src;
      payload.title = imgTitle;
      payload.kind = "image";
      if (src) payload.image_urls = [src];
      label = "Image";
      break;
    }
    case "tv-selection":
      payload.text = info.selectionText || "";
      payload.title = (info.selectionText || "").slice(0, 60);
      payload.kind = "text";
      payload.source = "selection";
      label = "Selection";
      break;
    case "tv-action-open":
      chrome.tabs.create({ url: serverUrl(cfg) + "/" });
      return;
    case "tv-action-settings":
      chrome.runtime.openOptionsPage();
      return;
    default:
      return;
  }
  if (!payload.url && !payload.text) {
    if (tab && tab.id) sendTabState(tab.id, { phase: "error", error: "nothing to save" });
    return;
  }
  const r = await saveWith(payload, label);
  if (tab && tab.id) {
    if (r.ok) sendTabState(tab.id, { phase: "saved", id: r.id || "", label });
    else sendTabState(tab.id, { phase: "error", error: r.error || "The item was not saved" });
  }
});
// ---------------------------------------------------------------------------
// Hotkey saves (content-script picks). Mirrors the context-menu payloads.
// ---------------------------------------------------------------------------

function pickPayload(msg) {
  const pick = msg.pick || {};
  const payload = { tags: msg.tags || [] };
  let label = "";
  if (pick.type === "image") {
    payload.url = pick.src || "";
    payload.title = (pick.alt || "saved image").slice(0, 200);
    payload.kind = "image";
    if (pick.src) payload.image_urls = [pick.src];
    label = "Image";
  } else {
    payload.url = pick.href || "";
    payload.title = (pick.text || "").slice(0, 200);
    payload.kind = "link";
    label = "Link";
  }
  return { payload, label };
}

async function savePick(msg, sender) {
  const tabId = sender && sender.tab && sender.tab.id;
  const pick = msg.pick || {};
  const cfg = await getConfig();

  // Image WITH page-fetched bytes -> multipart upload to /api/items/upload.
  // Private/self-hosted sources (Immich) bounce the vault's own download,
  // so the bytes ride along instead.
  if (pick.type === "image" && typeof pick.bytes === "string" && pick.bytes.startsWith("data:")) {
    return uploadImageStaged(cfg, tabId, {
      url: pick.src || "",
      title: (pick.alt || "saved image").slice(0, 200),
      tags: msg.tags || [],
      dataUrl: pick.bytes,
    });
  }

  const { payload, label } = pickPayload(msg);
  if (!payload.url) {
    if (tabId) sendTabState(tabId, { phase: "error", error: "nothing to save" });
    return { ok: false, error: "nothing to save" };
  }
  if (tabId) sendTabState(tabId, { phase: "saving", label });
  const r = await saveWith(payload, label);
  if (tabId) {
    if (r.ok) sendTabState(tabId, { phase: "saved", id: r.id || "", label });
    else sendTabState(tabId, { phase: "error", error: r.error || "The item was not saved" });
  }
  return r;
}

// Multipart image upload: the bytes were fetched by the content script with
// the page's own session, so private/self-hosted sources save correctly.
async function uploadImageStaged(cfg, tabId, { url, title, tags, dataUrl }) {
  const label = "Image";
  const tabSend = (state) => { if (tabId) sendTabState(tabId, state); };
  tabSend({ phase: "saving", label });
  try {
    const base = serverUrl(cfg);
    if (!base) throw new Error("set server URL in extension options");
    if (!(await ensureOrigin(base + "/"))) throw new Error("host permission denied");
    const blob = dataUrlToBlob(dataUrl);
    const form = new FormData();
    form.append("payload", JSON.stringify({ url, title, tags: tags || [] }));
    form.append("file", blob, "saved-image");
    const res = await fetch(base + "/api/items/upload", {
      method: "POST",
      headers: basicAuth(cfg) ? { Authorization: basicAuth(cfg) } : {},
      body: form,
    });
    if (res.status === 401) throw new Error("wrong credentials — update extension options");
    if (!res.ok) {
      let detail = res.statusText;
      try { detail = (await res.json()).detail || detail; } catch (e) {}
      throw new Error("HTTP " + res.status + " " + detail);
    }
    const data = await res.json();
    tabSend({ phase: "saved", id: data.id || "", label });
    return { ok: true, id: data.id };
  } catch (e) {
    const error = (e && e.message) || String(e);
    tabSend({ phase: "error", error });
    return { ok: false, error };
  }
}

// "data:image/png;base64,...." -> Blob for the multipart upload.
function dataUrlToBlob(dataUrl) {
  const m = /^data:([^;,]+)?(;base64)?,(.*)$/.exec(dataUrl);
  if (!m) throw new Error("bad data url");
  const mime = m[1] || "application/octet-stream";
  const body = m[3] || "";
  if (!m[2]) return new Blob([decodeURIComponent(body)], { type: mime });
  const bin = atob(body);
  const arr = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
  return new Blob([arr], { type: mime });
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg && msg.type === "tv-save-pick") {
    savePick(msg, sender)
      .then((r) => sendResponse(r))
      .catch((err) => sendResponse({ ok: false, error: (err && err.message) || String(err) }));
    return true;
  }
  if (msg && msg.type === "tv-save-tab") {
    (async () => {
      let tab = msg.tab || null;
      try {
        const [active] = await chrome.tabs.query({ active: true, currentWindow: true });
        if (active) {
          tab = {
            id: active.id,
            url: active.url || (tab && tab.url) || "",
            title: active.title || (tab && tab.title) || "",
          };
        }
      } catch (e) { /* fall back to the message tab */ }
      return saveTabPage(tab, msg.tags);
    })().then((r) => sendResponse(r)).catch((err) => {
      sendResponse({ ok: false, error: (err && err.message) ? err.message : String(err) });
    });
    return true; // async
  }
  if (msg && msg.type === "tv-open-vault") {
    getConfig().then((cfg) => {
      chrome.tabs.create({ url: serverUrl(cfg) + "/" });
      sendResponse({ ok: true });
    });
    return true;
  }
  if (msg && msg.type === "tv-add-tags") {
    (async () => {
      const itemId = (msg && msg.id) || "";
      const tags = Array.isArray(msg && msg.tags) ? msg.tags : [];
      if (!itemId) return { ok: false, error: "no item id" };
      const cfg = await getConfig();
      const base = serverUrl(cfg);
      if (!base) return { ok: false, error: "set server URL in extension options" };
      if (!(await ensureOrigin(base + "/"))) return { ok: false, error: "host permission denied" };
      try {
        const res = await fetch(base + "/api/items/" + encodeURIComponent(itemId) + "/tags", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            ...(basicAuth(cfg) ? { Authorization: basicAuth(cfg) } : {}),
          },
          body: JSON.stringify({ tags }),
        });
        if (res.status === 401) return { ok: false, error: "wrong credentials — update extension options" };
        if (!res.ok) {
          let detail = res.statusText;
          try { detail = (await res.json()).detail || detail; } catch (e) {}
          return { ok: false, error: "HTTP " + res.status + " " + detail };
        }
        const data = await res.json();
        return { ok: true, tags: (data && data.tags) || tags };
      } catch (e) {
        return { ok: false, error: "cannot reach server: " + e.message };
      }
    })().then(sendResponse).catch((err) => sendResponse({ ok: false, error: String(err) }));
    return true;
  }
  if (msg && msg.type === "tv-import-start") {
    startImport(msg.platforms || [], msg.redditUser || "", msg.pinUrl || "")
      .then((r) => sendResponse(r))
      .catch((e) => sendResponse({ ok: false, error: e.message }));
    return true;
  }
  if (msg && msg.type === "tv-import-status") {
    sendResponse(importJob ? importStatus() : null);
    return false;
  }
  if (msg && msg.type === "tv-import-stop") {
    stopImport();
    sendResponse({ ok: true });
    return false;
  }
  if (msg && msg.type === "tv-autosync-get") {
    getAutoSync().then((st) => sendResponse({ ok: true, autoSync: st })).catch((e) => sendResponse({ ok: false, error: String(e) }));
    return true;
  }
  if (msg && msg.type === "tv-autosync-save") {
    (async () => {
      const st = await getAutoSync();
      const next = msg.autoSync || {};
      st.enabled = !!next.enabled;
      st.intervalHours = Number(next.intervalHours) || 6;
      await saveAutoSync(st);
      await scheduleAutoSync();
      return { ok: true, autoSync: st };
    })().then((r) => sendResponse(r)).catch((e) => sendResponse({ ok: false, error: String(e) }));
    return true;
  }
  if (msg && msg.type === "tv-autosync-run") {
    autoSyncRun("manual").then((r) => sendResponse(r)).catch((e) => sendResponse({ ok: false, error: String(e) }));
    return true;
  }
});

// ---------------------------------------------------------------------------
// Auto-sync: run the same saved-content pull on a schedule (chrome.alarms).
// Manual "pull now" is untouched — this just keeps the vault updated.
// ---------------------------------------------------------------------------

const AUTOSYNC_ALARM = "tv-autosync";
const AUTOSYNC_KEY = "tv-autosync";
const AUTOSYNC_MIN_HOURS = 1;

async function getAutoSync() {
  const stored = await chrome.storage.local.get({ "tv-autosync": null });
  return stored["tv-autosync"] || { enabled: false, intervalHours: 6, lastSync: "", lastResult: "" };
}

async function saveAutoSync(st) {
  await chrome.storage.local.set({ "tv-autosync": st });
}

// (Re)schedule the alarm from the stored interval. Call after any settings save.
async function scheduleAutoSync() {
  const st = await getAutoSync();
  await chrome.alarms.clear(AUTOSYNC_ALARM);
  if (!st.enabled) return;
  const hours = Math.max(AUTOSYNC_MIN_HOURS, Number(st.intervalHours) || 6);
  await chrome.alarms.create(AUTOSYNC_ALARM, {
    periodInMinutes: hours * 60,
    // fire the first one a minute after (re)enable so the user sees it work
    delayInMinutes: 1,
  });
}

// Wall-clock guard: alarms can drift/double-fire on browser sleep. Never run
// twice inside the configured interval.
async function autoSyncDue(st) {
  if (!st.lastSync) return true;
  const last = Date.parse(st.lastSync);
  if (Number.isNaN(last)) return true;
  const hours = Math.max(AUTOSYNC_MIN_HOURS, Number(st.intervalHours) || 6);
  return Date.now() - last >= hours * 3600 * 1000;
}

async function autoSyncRun(reason) {
  const st = await getAutoSync();
  if (!st.enabled && reason !== "manual") {
    return { ok: false, error: "auto-sync is off" };
  }
  if (importJob && importJob.running) {
    return { ok: false, error: "a pull is already running" };
  }
  if (reason !== "manual" && !(await autoSyncDue(st))) {
    return { ok: false, error: "not due yet" };
  }
  const cfg = await getConfig();
  const sources = cfg.sources || {};
  const platforms = ["twitter", "reddit", "instagram", "pinterest"].filter(
    (p) => sources[p] && sources[p].enabled
  );
  if (!platforms.length) {
    return { ok: false, error: "no enabled sources" };
  }
  // Reuse the exact manual-pull pipeline: same scrapers, same dedupe, same
  // stop conditions (consecutive dupes = everything new is already saved).
  const started = await startImport(platforms, "", "", { auto: true });
  if (!started.ok) {
    st.lastResult = "failed: " + (started.error || "unknown");
    await saveAutoSync(st);
    return started;
  }
  const stamp = new Date().toISOString();
  st.lastSync = stamp;
  st.lastResult = "started " + stamp;
  await saveAutoSync(st);
  return { ok: true };
}

// Called when the import runner settles — record the outcome + notify once.
async function autoSyncFinish() {
  const st = await getAutoSync();
  const j = importJob;
  if (!j) return;
  st.lastSync = new Date().toISOString();
  st.lastResult = j.aborted
    ? "aborted"
    : `+${j.created} new, ${j.dupes} dupes, ${j.failed} failed`;
  await saveAutoSync(st);
  const summary = `auto-sync: +${j.created} new · ${j.dupes} already saved` +
    (j.failed ? ` · ${j.failed} failed` : "");
  notify("Taste Engine", summary);
}

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm && alarm.name === AUTOSYNC_ALARM) {
    autoSyncRun("alarm").catch(() => {});
  }
});

chrome.runtime.onStartup.addListener(() => {
  scheduleAutoSync().catch(() => {});
});

chrome.runtime.onInstalled.addListener(() => {
  scheduleAutoSync().catch(() => {});
});

// ---------------------------------------------------------------------------
// Saved-content puller (merged from Social Exporter). Pulls bookmarks/saved
// posts from the logged-in session and POSTs each one to the vault; the vault
// dedupes by canonical source key and answers with a `created` flag.
// ---------------------------------------------------------------------------

let importJob = null; // {platforms, running, aborted, pulled, created, dupes, failed, errors[], startedAt}
let importController = null;

function importStatus() {
  if (!importJob) return null;
  const j = importJob;
  return {
    platforms: j.platforms,
    running: j.running,
    aborted: j.aborted,
    pulled: j.pulled,
    created: j.created,
    dupes: j.dupes,
    failed: j.failed,
    errors: j.errors.slice(0, 8),
    startedAt: j.startedAt,
    stopReason: j.stopReason || "",
  };
}

async function impExists(url, cfg) {
  const base = serverUrl(cfg);
  if (!base || !url) return false;
  try {
    const res = await fetch(base + "/api/items/exists?url=" + encodeURIComponent(url), {
      headers: basicAuth(cfg) ? { Authorization: basicAuth(cfg) } : {},
    });
    if (!res.ok) return false;
    const data = await res.json();
    return !!data.exists;
  } catch (e) {
    return false;
  }
}

async function impSave(payload, cfg) {
  const base = serverUrl(cfg);
  if (!base) return { ok: false, error: "set server URL in extension options" };
  if (!(await ensureOrigin(base + "/"))) {
    return { ok: false, error: "host permission denied" };
  }
  try {
    const res = await fetch(base + "/api/items", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(basicAuth(cfg) ? { Authorization: basicAuth(cfg) } : {}),
      },
      body: JSON.stringify(payload),
    });
    if (res.status === 401) return { ok: false, error: "wrong credentials — update extension options" };
    if (!res.ok) {
      let detail = res.statusText;
      try { detail = (await res.json()).detail || detail; } catch (e) {}
      return { ok: false, error: "HTTP " + res.status + " " + detail };
    }
    const data = await res.json();
    return { ok: true, created: !!data.created, id: data.id };
  } catch (e) {
    return { ok: false, error: "cannot reach server: " + e.message };
  }
}

function impTags(v) {
  return String(v || "").split(/,\s*/).map((t) => t.trim()).filter(Boolean).slice(0, 10);
}

function impImageUrls(item) {
  const list = Array.isArray(item.image_urls) ? item.image_urls.filter((u) => u) : [];
  if (list.length) return list;
  return item.image_url ? [item.image_url] : [];
}

function impPayloadFor(platform, item) {
  const base = {
    url: item.url || "",
    title: (item.title || "").slice(0, 400),
    text: (item.description || "").slice(0, 4000),
    tags: impTags(item.tags),
  };
  switch (platform) {
    case "twitter":
      return { ...base, kind: "link", source: "x", image_urls: impImageUrls(item) };
    case "reddit":
      return { ...base, kind: "link", source: "reddit", image_urls: impImageUrls(item) };
    case "instagram": {
      const video = item.media_type === "video";
      return {
        ...base,
        kind: video ? "video" : "image",
        source: "instagram",
        image_urls: video ? [] : impImageUrls(item),
        video_url: video && item.image_url ? item.image_url : "",
      };
    }
    case "pinterest":
      // url must be the canonical pin URL so the vault dedupes pins.
      return {
        ...base,
        url: item.pin_url || item.url || "",
        kind: "image",
        source: "pinterest",
        image_urls: impImageUrls(item),
      };
    default:
      return { ...base, kind: "link" };
  }
}

async function impRunTask(task, cfg, controller) {
  // Scrape inside a platform tab via the content-script scraper (same-origin,
  // uses the user's logged-in session). Returns normalized items.
  let items;
  try {
    items = await scrapeTask(task, controller);
  } catch (e) {
    throw new Error((e && e.message) || String(e));
  }
  const MIN_PULLED = 50;   // don't trigger on the first items
  const CONCURRENCY = 4;
  const queue = (items || []).slice();
  let stop = false;

  async function saveOne(item) {
    if (controller.signal.aborted || stop) return;
    const payload = impPayloadFor(task.platform, item);
    if (!payload.url && !payload.text) return;
    if (payload.url && await impExists(payload.url, cfg)) {
      importJob.dupes += 1;
      importJob.consecutiveDupes = (importJob.consecutiveDupes || 0) + 1;
      importJob.pulled += 1;
      return;
    }
    const r = await impSave(payload, cfg);
    if (controller.signal.aborted || stop) return;
    if (r.ok) {
      if (r.created) {
        importJob.created += 1;
        importJob.consecutiveDupes = 0;
      } else {
        importJob.dupes += 1;
        importJob.consecutiveDupes = (importJob.consecutiveDupes || 0) + 1;
      }
    } else {
      importJob.failed += 1;
      if (importJob.errors.length < 8) {
        importJob.errors.push((payload.url || "?") + " — " + r.error);
      }
      if (r.error && /credential|credentials|401/.test(r.error) && importJob.failed >= 3) {
        importJob.result = "auth";
        stop = true;
        throw new Error(r.error);
      }
    }
    importJob.pulled += 1;
  }

  async function worker() {
    while (queue.length && !controller.signal.aborted && !stop) {
      const item = queue.shift();
      await saveOne(item);
      if (importJob.consecutiveDupes >= 5) {
        importJob.stopReason = "dupes";
        stop = true;
        break;
      }
      if (importJob.pulled >= MIN_PULLED && importJob.created / importJob.pulled < cfg.stopRatio) {
        importJob.stopReason = "ratio";
        stop = true;
        break;
      }
      if (importJob.pulled % 10 === 0) persistImportJob();
    }
  }

  const workers = [];
  for (let i = 0; i < CONCURRENCY; i++) workers.push(worker());
  await Promise.all(workers);
}

async function startImport(platforms, redditUser, pinUrl, opts = {}) {
  if (importJob && importJob.running) return { ok: false, error: "import already running" };
  const cfg = await getConfig();
  const controller = new AbortController();
  importController = controller;
  const sources = cfg.sources || {};
  const tasks = [];
  if (platforms.includes("twitter") && sources.twitter && sources.twitter.enabled) tasks.push({ platform: "twitter", type: "bookmarks", url: (sources.twitter.url || "").trim() || undefined });
  if (platforms.includes("reddit") && sources.reddit && sources.reddit.enabled) {
    const u = (sources.reddit.url || "").trim();
    const m = u.match(/^\S*\/user\/([^/]+)\/saved/);
    if (!m) return { ok: false, error: "reddit URL must be your saved page (…/user/<name>/saved/)" };
    tasks.push({ platform: "reddit", type: "user_saved", username: m[1] });
  }
  if (platforms.includes("instagram") && sources.instagram && sources.instagram.enabled) {
    const u = (sources.instagram.url || "").trim();
    const m = u.match(/^\S*\/([\w.]+)\/saved/);
    if (!m) return { ok: false, error: "instagram URL must be your saved page (…/<username>/saved/)" };
    tasks.push({ platform: "instagram", type: "saved_posts", username: m[1], url: (sources.instagram.url || "").trim() });
  }
  if (platforms.includes("pinterest") && sources.pinterest && sources.pinterest.enabled) {
    const u = (sources.pinterest.url || "").trim();
    if (!/^\S*pinterest\.(com|ca)|^\S*pin\.it\//.test(u)) return { ok: false, error: "pinterest URL is not a pinterest URL" };
    const pinMatch = u.match(/\/pin\/(\d+)/);
    if (pinMatch) {
      tasks.push({ platform: "pinterest", type: "pin", id: pinMatch[1], url: (sources.pinterest.url || "").trim() });
    } else {
      const parts = u.split("/").filter(Boolean).filter((p) => p !== "https:" && p !== "http:");
      // https://www.pinterest.com/<username>/<board-slug>/ or /_pins/ etc.
      if (parts.length >= 2) {
        const base = parts[parts.length - 2];
        const leaf = parts[parts.length - 1];
        if (leaf === "_created") tasks.push({ platform: "pinterest", type: "user_created", username: base, url: u });
        else if (leaf === "_pins") tasks.push({ platform: "pinterest", type: "user_pins", username: base, url: u });
        else if (base && !leaf.startsWith("_")) tasks.push({ platform: "pinterest", type: "board", username: base, boardSlug: leaf, url: u });
        else if (base && leaf.startsWith("_")) tasks.push({ platform: "pinterest", type: "user_pins", username: base, url: u });
        else tasks.push({ platform: "pinterest", type: "profile", username: base || leaf, url: u });
      } else if (parts.length === 1) {
        tasks.push({ platform: "pinterest", type: "profile", username: parts[0], url: u });
      } else {
        return { ok: false, error: "pinterest URL is not a pinterest URL" };
      }
    }
  }
  if (tasks.length === 0) return { ok: false, error: "no enabled sources — configure them in settings" };

  importJob = {
    platforms: [...platforms], running: true, aborted: false,
    pulled: 0, created: 0, dupes: 0, failed: 0, errors: [],
    startedAt: new Date().toISOString(),
    auto: !!(opts && opts.auto),
  };
  persistImportJob();

  (async () => {
    for (let i = 0; i < tasks.length; i++) {
      if (controller.signal.aborted) break;
      try {
        await impRunTask(tasks[i], cfg, controller);
      } catch (e) {
        importJob.failed += 1;
        if (importJob.errors.length < 8) {
          importJob.errors.push(tasks[i].platform + " — " + e.message);
        }
      }
      if (i < tasks.length - 1 && !controller.signal.aborted) {
        await sleep(1000 + Math.random() * 1000);
      }
    }
    importJob.running = false;
    importJob.aborted = controller.signal.aborted;
    persistImportJob();
    if (importJob.auto) {
      autoSyncFinish().catch(() => {});
    }
  })();
  return { ok: true };
}

function stopImport() {
  if (importController) importController.abort();
  if (importJob && importJob.running) importJob.aborted = true;
}

function persistImportJob() {
  chrome.storage.local.set({ "tv-import-job": importStatus() }).catch(() => {});
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ---------------------------------------------------------------------------
// Same-origin scraping dispatch: inject scraper.js into a platform tab, ask it
// to run the platform's own internal APIs (cookies + no CORS), return items.
// ---------------------------------------------------------------------------

const PLATFORM_HOME = {
  twitter: "https://x.com/i/history",
  reddit: "https://www.reddit.com/",
  instagram: "https://www.instagram.com/",
  pinterest: "https://www.pinterest.com/",
};

function tabPatterns(platform) {
  switch (platform) {
    case "twitter": return ["*://*.x.com/*", "*://*.twitter.com/*"];
    case "reddit": return ["*://*.reddit.com/*", "*://*.redd.it/*"];
    case "instagram": return ["*://*.instagram.com/*"];
    case "pinterest": return ["*://*.pinterest.com/*", "*://*.pin.it/*"];
    default: return [];
  }
}

function waitForTabComplete(tabId, timeoutMs = 30000) {
  return new Promise((resolve) => {
    const start = Date.now();
    const iv = setInterval(() => {
      chrome.tabs.get(tabId).then((t) => {
        if (t.status === "complete" || Date.now() - start > timeoutMs) {
          clearInterval(iv);
          resolve();
        }
      }).catch(() => {
        clearInterval(iv);
        resolve();
      });
    }, 400);
  });
}

// A background (active:false) tab can still be settling after "complete":
// layout, redirects, and lazy embeds. sleep() below gives it a beat.
// Returns {tabId, fresh} — fresh tabs get closed once the task settles.
async function tabForTask(task) {
  const existing = await chrome.tabs.query({ url: tabPatterns(task.platform) });
  if (existing && existing.length) {
    const alive = await chrome.tabs.get(existing[0].id).then(() => true).catch(() => false);
    if (alive) return { tabId: existing[0].id, fresh: false };
    // Stale/disarded tab (browser slept it): drop it and open fresh below.
  }
  // A real target page (the configured board/saved/profile URL) attaches far
  // more reliably than the bare home feed, which can sit on a login/wall.
  let url = task.url || PLATFORM_HOME[task.platform];
  if (task.platform === "reddit" && task.username) {
    url = `https://www.reddit.com/user/${encodeURIComponent(task.username)}/saved/`;
  }
  const tab = await chrome.tabs.create({ url, active: false });
  await waitForTabComplete(tab.id);
  await sleep(1500);
  return { tabId: tab.id, fresh: true };
}

async function closeTabSoon(tabId) {
  // Give the tab a beat (stray fetches settle), then remove it. Best effort.
  setTimeout(() => {
    chrome.tabs.remove(tabId).catch(() => {});
  }, 1500);
}

async function scrapeTask(task, controller) {
  const { tabId, fresh } = await tabForTask(task);
  try {
    // The scraper content script must be attached before the tv-scrape message
    // fires. A fresh background tab may still be loading, and Zen/Firefox can
    // silently skip executeScript for ~seconds after "complete" — retry the
    // injection+ping loop instead of one fire-and-forget attempt. Late
    // attempts briefly ACTIVATE the tab: suspended background tabs skip
    // script injection until they are foregrounded once.
    let attached = false;
    let lastErr = "";
    for (let attempt = 0; attempt < 8 && !attached; attempt++) {
      if (controller.signal.aborted) throw new Error("aborted before scrape");
      if (attempt >= 5 && fresh) {
        await chrome.tabs.update(tabId, { active: true }).catch(() => {});
        await sleep(800);
      }
      try {
        await chrome.scripting.executeScript({ target: { tabId }, files: ["scraper.js"] });
      } catch (e) {
        lastErr = (e && e.message) || String(e);
      }
      try {
        await chrome.tabs.sendMessage(tabId, { action: "tv-ping" });
        attached = true;
      } catch (e) {
        lastErr = (e && e.message) || String(e);
        await sleep(1500 * (attempt + 1));
      }
    }
    if (!attached) {
      throw new Error("could not attach to a " + task.platform + " tab (fresh open) — last error: " + lastErr);
    }
    const res = await chrome.tabs.sendMessage(tabId, { action: "tv-scrape", task });
    if (!res) throw new Error("no response from page scraper — did the tab load?");
    if (!res.ok) throw new Error(res.error || "scrape failed");
    return res.items || [];
  } finally {
    // Opened it for this task? Close it once the task settles — synced or failed.
    if (fresh) closeTabSoon(tabId);
  }
}
