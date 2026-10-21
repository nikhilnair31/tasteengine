// Taste Engine popup.
// First run: setup view (server URL + login, test button). After setup:
// a two-button action view (save this page / open engine).
"use strict";

const $ = (id) => document.getElementById(id);

const CFG_DEFAULTS = { server: "", user: "", pass: "", tags: [] };
const DEFAULT_SERVER = "http://127.0.0.1:8770";
const DEFAULT_SOURCES = {
  twitter: { enabled: true, url: "https://x.com/i/history" },
  reddit: { enabled: false, url: "" },
  instagram: { enabled: false, url: "" },
  pinterest: { enabled: false, url: "" },
};

function readCfg() {
  return chrome.storage.local.get(CFG_DEFAULTS);
}

function normServer(s) {
  return (s || "").trim().replace(/\/+$/, "");
}

function showView(id) {
  $("view-main").hidden = id !== "view-main";
  $("view-setup").hidden = id !== "view-setup";
}

function setStatus(id, text, cls) {
  const el = $(id);
  el.textContent = text;
  el.className = "status" + (cls ? " " + cls : "");
}

// First run: persist sensible defaults so the main view has something to act
// on and the settings page is prefilled, instead of forcing the login card.
async function bootstrapIfEmpty() {
  const cfg = await readCfg();
  if (cfg.server) return;
  await chrome.storage.local.set({
    server: DEFAULT_SERVER,
    sources: DEFAULT_SOURCES,
  });
}

async function fillSetup() {
  const cfg = await readCfg();
  $("server").value = cfg.server || DEFAULT_SERVER;
  $("user").value = cfg.user || "";
  $("pass").value = cfg.pass || "";
}

(async function init() {
  await bootstrapIfEmpty();
  showView("view-main");
  // Icon + label for each stacked action.
  $("save-tab").textContent = "save this page";
  $("open-vault").textContent = "open engine";
  $("open-settings").textContent = "open settings";
  initImp();
  initHotkeyHint();
})();

// Small line under the actions showing the live hotkeys; edited in settings.
async function initHotkeyHint() {
  const el = $("hk-hint");
  if (!el) return;
  try {
    const cfg = await chrome.storage.local.get({ hotkeyImage: "alt+shift+s", hotkeyLink: "alt+shift+l" });
    const label = (s) => String(s || "").trim().toLowerCase().split("+").filter(Boolean)
      .map((p) => (p === "ctrl" || p === "meta") ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1))
      .join("+");
    const parts = [];
    if (cfg.hotkeyImage) parts.push(`<b>${label(cfg.hotkeyImage)}</b> image`);
    if (cfg.hotkeyLink) parts.push(`<b>${label(cfg.hotkeyLink)}</b> link`);
    el.innerHTML = parts.length ? parts.join(" &middot; ") : "";
  } catch (e) { /* hint is optional */ }
}

// Show a flat toast on the saved page, then close the popup.
function paintToast(text, bg, fg) {
  const d = document.createElement("div");
  d.textContent = text;
  const s = d.style;
  s.position = "fixed";
  s.bottom = "20px";
  s.right = "20px";
  s.background = bg;
  s.color = fg;
  s.padding = "10px 16px";
  s.borderRadius = "0";
  s.fontSize = "13px";
  s.fontFamily = "system-ui,sans-serif";
  s.zIndex = "9999999";
  s.opacity = "1";
  s.transition = "opacity .25s";
  s.pointerEvents = "none";
  s.border = "1px solid " + fg;
  document.body.appendChild(d);
  setTimeout(() => {
    d.style.opacity = "0";
    setTimeout(() => d.remove(), 250);
  }, 2000);
}

function pageToast(tabId, ok, msg) {
  const bg = "#272124";
  const fg = ok ? "#8fbf9f" : "#ef5b57";
  chrome.scripting.executeScript({
    target: { tabId },
    func: paintToast,
    args: [(ok ? "\u2713 " : "\u2717 ") + msg, bg, fg],
  }).catch(() => {});
}

// Setup view: test the server + login, save on success, switch to main view.
$("btn-test").addEventListener("click", async () => {
  const server = normServer($("server").value);
  const user = $("user").value.trim();
  const pass = $("pass").value;
  if (!server) {
    setStatus("sstatus", "enter a server URL", "bad");
    return;
  }
  let url;
  try {
    url = new URL(server);
  } catch (e) {
    setStatus("sstatus", "invalid server URL", "bad");
    return;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    setStatus("sstatus", "server URL must start with http:// or https://", "bad");
    return;
  }
  setStatus("sstatus", "testing\u2026", "");
  // Ask for host permission to the vault origin. No-op when granted already.
  try {
    const granted = await chrome.permissions.request({ origins: [url.origin + "/*"] });
    if (granted === false) {
      setStatus("sstatus", "host permission denied", "bad");
      return;
    }
  } catch (e) {}
  try {
    const res = await fetch(server + "/api/meta", {
      headers: user ? { Authorization: "Basic " + btoa(user + ":" + pass) } : {},
      cache: "no-store",
    });
    if (res.status === 401) {
      setStatus("sstatus", "wrong credentials", "bad");
      return;
    }
    if (!res.ok) {
      setStatus("sstatus", "HTTP " + res.status, "bad");
      return;
    }
    const meta = await res.json();
    await chrome.storage.local.set({ server, user, pass });
    showView("view-main");
    setStatus("status", "connected \u2014 " + meta.total + " items \u00b7 settings saved", "ok");
  } catch (e) {
    setStatus("sstatus", "cannot reach server", "bad");
  }
});

// main: settings buttons -> open the full settings page (separate tab/popout).
async function openSettings() {
  try {
    await chrome.runtime.openOptionsPage();
  } catch (e) {
    // Fallback: inline setup view if the options page can't open.
    await fillSetup();
    setStatus("sstatus", "", "");
    showView("view-setup");
  }
}

// main: open settings opens the settings popout (sources + stop% + pull).
$("open-settings").addEventListener("click", openSettings);

// main: save the active tab.
$("save-tab").addEventListener("click", async () => {
  setStatus("status", "saving\u2026");
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (!tab || !tab.url) {
      setStatus("status", "no active tab", "bad");
      return;
    }
    const cfg = await readCfg();
    const SAVE_TIMEOUT_MS = 20000;
    const r = await Promise.race([
      chrome.runtime.sendMessage({
        type: "tv-save-tab",
        tab: { url: tab.url, title: tab.title },
        tags: cfg.tags || [],
      }),
      new Promise((_, reject) =>
        setTimeout(() => reject(new Error("timed out waiting for the engine")), SAVE_TIMEOUT_MS)
      ),
    ]);
    const ok = !!(r && r.ok);
    const msg = ok
      ? "Page saved"
      : (r && r.error ? r.error : "failed \u2014 check the engine");
    if (ok) {
      setStatus("status", "saved \u2713", "ok");
    } else {
      setStatus("status", msg, "bad");
    }
    if (tab.id) pageToast(tab.id, ok, msg);
    if (ok) {
      // Brief confirmation so the popup does not vanish instantly.
      setTimeout(() => window.close(), 600);
    }
  } catch (err) {
    setStatus(
      "status",
      "save \u2014 " + (err && err.message ? err.message : "no response from the engine"),
      "bad"
    );
  }
});

// main: open the vault in a tab.
$("open-vault").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "tv-open-vault" });
  window.close();
});

// ---------------------------------------------------------------------------
// Pull saved posts (merged Social Exporter)
// ---------------------------------------------------------------------------

const IMP_STATUS = $("imp-status");
let impPoll = null;

function impSummary(st) {
  if (!st) return "";
  let out = `${st.pulled} pulled · ${st.created} new · ${st.dupes} already saved`;
  if (st.failed) out += ` · ${st.failed} failed`;
  if (st.errors && st.errors.length) out += ` — ${st.errors[0]}`;
  return out;
}

function impSet(msg, cls) {
  IMP_STATUS.textContent = msg || "";
  IMP_STATUS.className = "imp-status" + (cls ? " " + cls : "");
}

function stopPolling() {
  if (impPoll) clearInterval(impPoll);
  impPoll = null;
}

async function pollImp() {
  let st = null;
  try {
    st = await chrome.runtime.sendMessage({ type: "tv-import-status" });
  } catch (e) {
    stopPolling();
    impSet("cannot reach background", "bad");
    return;
  }
  if (!st) return;
  if (st.running) {
    impSet("pulling… " + impSummary(st), "");
    return;
  }
  stopPolling();
  const why = st.stopReason === "dupes"
    ? " — stopped: hit already-saved posts (newest-first)"
    : st.stopReason === "ratio"
      ? " — stopped: new items fell below 20% of the pull"
      : (st.aborted ? " — stopped" : " — done");
  impSet(impSummary(st) + why, st.failed ? "bad" : "ok");
}

// Restore a running/stale import summary; the pull controls live in the
// options-page popout (pull data / server settings open it).
async function initImp() {
  const stored = await chrome.storage.local.get({ "tv-import-job": null });
  const last = stored["tv-import-job"];
  if (last) {
    if (last.running) {
      stopPolling();
      impPoll = setInterval(pollImp, 1000);
      impSet("pulling… " + impSummary(last), "");
    } else if (last.pulled || last.created || last.dupes || last.failed) {
      impSet(impSummary(last) + (last.aborted ? " — stopped" : " — done"), last.failed ? "bad" : "ok");
    }
  }
}