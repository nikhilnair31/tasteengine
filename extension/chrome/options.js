// Taste Engine options page — server, pull sources, and pull trigger.
"use strict";

const $ = (id) => document.getElementById(id);
const DEFAULTS = ["http://127.0.0.1:8770"];

const SRC_MAP = [
  { platform: "twitter", check: "src-x", url: "src-x-url", def: "https://x.com/i/history" },
  { platform: "reddit", check: "src-reddit", url: "src-reddit-url", def: "" },
  { platform: "instagram", check: "src-ig", url: "src-ig-url", def: "" },
  { platform: "pinterest", check: "src-pin", url: "src-pin-url", def: "" },
];

function setStatus(elId, text, cls) {
  const el = $(elId);
  el.textContent = text;
  el.className = "status" + (cls ? " " + cls : "");
}

function normServer(s) {
  return (s || "").trim().replace(/\/+$/, "");
}

function defaultSources() {
  const out = {};
  for (const s of SRC_MAP) out[s.platform] = { enabled: s.platform === "twitter", url: s.def };
  return out;
}

function readSourcesFields() {
  const out = {};
  for (const s of SRC_MAP) {
    out[s.platform] = { enabled: $(s.check).checked, url: $(s.url).value.trim() };
  }
  return out;
}

function applySourcesFields(sources) {
  const src = sources || defaultSources();
  for (const s of SRC_MAP) {
    const v = src[s.platform] || {};
    $(s.check).checked = !!v.enabled;
    $(s.url).value = v.url !== undefined ? v.url : s.def;
  }
}

(async function init() {
  const cfg = await chrome.storage.local.get({
    server: DEFAULTS[0],
    user: "",
    pass: "",
    tags: [],
    sources: null,
    stopRatio: 0.20,
  });
  $("server").value = cfg.server;
  $("user").value = cfg.user;
  $("pass").value = cfg.pass;
  applySourcesFields(cfg.sources);
  // Load the pull-stop threshold into the control.
  const ratioSel = $("stop-ratio");
  const saved = Number(cfg.stopRatio);
  if (saved && ratioSel.querySelector(`option[value="${saved}"]`)) {
    ratioSel.value = String(saved);
  } else {
    ratioSel.value = "0.20";
  }
  ratioSel.addEventListener("change", () => {
    chrome.storage.local.set({ stopRatio: Number(ratioSel.value) }).catch(() => {});
  });
  restoreImp();
})();

async function saveAll() {
  const prior = await chrome.storage.local.get({ tags: [] });
  await chrome.storage.local.set({
    server: normServer($("server").value),
    user: $("user").value.trim(),
    pass: $("pass").value,
    tags: prior.tags || [],
    sources: readSourcesFields(),
    stopRatio: Number($("stop-ratio").value),
  });
}

// ---------------------------------------------------------------------------
// Editable hotkeys. Stored as combo strings ("alt+shift+s", "" = off) in
// chrome.storage.local (hotkeyImage / hotkeyLink). The content script in
// every tab listens for these — MV3 has no API to rebind chrome.commands.
// ---------------------------------------------------------------------------

const HK_FIELDS = [
  { key: "hotkeyImage", input: "hk-image", clear: "hk-image-clear" },
  { key: "hotkeyLink", input: "hk-link", clear: "hk-link-clear" },
];

function hkLabel(s) {
  return String(s || "").trim().toLowerCase().split("+").filter(Boolean)
    .map((p) => (p === "ctrl" || p === "meta") ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1))
    .join(" + ");
}

function hkComboString(e) {
  const parts = [];
  if (e.altKey) parts.push("alt");
  if (e.ctrlKey) parts.push("ctrl");
  if (e.metaKey) parts.push("meta");
  if (e.shiftKey) parts.push("shift");
  let k = String(e.key || "").toLowerCase();
  if (!/^[a-z0-9]$/.test(k) && /^key([a-z0-9])$/i.test(e.code || "")) k = e.code.slice(3).toLowerCase();
  if (k === "esc") k = "escape";
  if (k === "space") k = "space";
  if (/^(control|ctrl|alt|option|meta|cmd|command|shift|hyper|super|fn|numlock|capslock|scrolllock)$/.test(k)) return null;
  if (k && !parts.includes(k)) parts.push(k);
  return parts.length ? parts.join("+") : null;
}

function hkRender() {
  for (const f of HK_FIELDS) {
    // value re-read from the input's dataset mirror, set at load + after save
    const s = $(f.input).dataset.combo || "";
    $(f.input).value = s ? hkLabel(s) : "off";
  }
}

let hkRecording = null; // field config currently recording

function hkStopRecording(save) {
  if (!hkRecording) return;
  $(hkRecording.input).classList.remove("recording");
  hkRecording = null;
  if (!save) hkRender();
}

async function hkSave(field, combo) {
  const others = HK_FIELDS.filter((f) => f !== field);
  if (combo && others.some((f) => ($(f.input).dataset.combo || "") === combo)) {
    setStatus("hk-status", "that combo is already used by the other shortcut", "bad");
    return;
  }
  $(field.input).dataset.combo = combo || "";
  hkRender();
  try {
    await chrome.storage.local.set({ [field.key]: combo || "" });
    try {
      await chrome.runtime.sendMessage({ type: "tv-hotkeys-changed" });
    } catch (e) {}
    setStatus("hk-status", combo ? `saved — ${hkLabel(combo)}` : "saved — shortcut off", "ok");
  } catch (e) {
    setStatus("hk-status", "could not save the shortcut", "bad");
  }
}

for (const f of HK_FIELDS) {
  $(f.clear).addEventListener("click", async () => {
    hkStopRecording(false);
    await hkSave(f, "");
  });
  $(f.input).addEventListener("focus", () => {
    hkStopRecording(false);
    hkRecording = f;
    $(f.input).classList.add("recording");
    $(f.input).value = "press keys\u2026";
    setStatus("hk-status", "", "");
  });
  $(f.input).addEventListener("blur", () => hkStopRecording(false));
}

document.addEventListener("keydown", (e) => {
  if (!hkRecording) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") { hkStopRecording(false); return; }
  if (e.key === "Backspace") {
    const f = hkRecording;
    hkStopRecording(false);
    hkSave(f, "");
    return;
  }
  const combo = hkComboString(e);
  if (!combo) return; // bare modifier — wait for the full combo
  if (!e.altKey && !e.ctrlKey && !e.metaKey) {
    setStatus("hk-status", "combo must include alt, ctrl, or cmd", "bad");
    return;
  }
  const field = hkRecording;
  hkStopRecording(false);
  hkSave(field, combo);
}, { capture: true });

(async function initHotkeys() {
  const cfg = await chrome.storage.local.get({ hotkeyImage: "alt+shift+s", hotkeyLink: "alt+shift+l" });
  $("hk-image").dataset.combo = cfg.hotkeyImage || "";
  $("hk-link").dataset.combo = cfg.hotkeyLink || "";
  hkRender();
})();

// Single shared "save settings" — persists server + sources + stop ratio.
$("save-all").addEventListener("click", async () => {
  await saveAll();
  const src = readSourcesFields();
  const n = Object.values(src).filter((s) => s.enabled && s.url).length;
  setStatus("sources-status", n ? `saved — ${n} source${n === 1 ? "" : "s"} enabled` : "saved — no sources enabled", n ? "ok" : "");
});

$("test").addEventListener("click", async () => {
  const server = normServer($("server").value);
  const user = $("user").value.trim();
  const pass = $("pass").value;
  if (!server) { setStatus("status", "enter a server URL", "bad"); return; }
  try {
    const granted = await chrome.permissions.request({ origins: [new URL(server).origin + "/*"] });
    if (granted === false) { setStatus("status", "host permission denied", "bad"); return; }
  } catch (e) {}
  try {
    const res = await fetch(server + "/api/meta", {
      headers: user ? { Authorization: "Basic " + btoa(user + ":" + pass) } : {},
      cache: "no-store",
    });
    if (res.status === 401) { setStatus("status", "wrong credentials", "bad"); return; }
    if (!res.ok) { setStatus("status", "HTTP " + res.status, "bad"); return; }
    const meta = await res.json();
    // Credentials are valid — save them so pulls use the same login.
    await chrome.storage.local.set({
      server,
      user,
      pass,
    });
    setStatus("status", "connected — " + meta.total + " items in vault — saved", "ok");
  } catch (e) {
    setStatus("status", "cannot reach server", "bad");
  }
});

// ---------------------------------------------------------------------------
// Pull trigger + live status
// ---------------------------------------------------------------------------

let impPoll = null;

function impSummary(st) {
  if (!st) return "";
  let out = `${st.pulled} pulled · ${st.created} new · ${st.dupes} already saved`;
  if (st.failed) out += ` · ${st.failed} failed`;
  if (st.errors && st.errors.length) out += ` — ${st.errors[0]}`;
  return out;
}

function impSet(msg, cls) {
  setStatus("imp-status", msg, cls);
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
    $("pull").hidden = false;
    $("stop").hidden = true;
    return;
  }
  if (!st) return;
  if (st.running) {
    $("pull").hidden = true;
    $("stop").hidden = false;
    impSet("pulling… " + impSummary(st), "");
    return;
  }
  stopPolling();
  $("pull").hidden = false;
  $("stop").hidden = true;
  const why = st.stopReason === "dupes"
    ? " — stopped: hit already-saved posts (newest-first)"
    : st.stopReason === "ratio"
      ? " — stopped: new items fell below 20% of the pull"
      : (st.aborted ? " — stopped" : " — done");
  impSet(impSummary(st) + why, st.failed ? "bad" : "ok");
}

$("pull").addEventListener("click", async () => {
  try {
    // Persist anything the user edited on screen (server, login, sources) so
    // the background uses exactly what is visible, not stale stored creds.
    await saveAll();
    const fields = readSourcesFields();
    const enabled = SRC_MAP.filter((s) => fields[s.platform] && fields[s.platform].enabled)
      .map((s) => s.platform);
    const r = await chrome.runtime.sendMessage({
      type: "tv-import-start",
      platforms: enabled,
    });
    if (r && r.ok) {
      impSet("pulling…", "");
      $("pull").hidden = true;
      $("stop").hidden = false;
      stopPolling();
      impPoll = setInterval(pollImp, 1000);
    } else {
      impSet((r && r.error) || "failed to start", "bad");
    }
  } catch (e) {
    impSet("cannot reach background", "bad");
  }
});

$("stop").addEventListener("click", async () => {
  try {
    await chrome.runtime.sendMessage({ type: "tv-import-stop" });
  } catch (e) {}
  impSet("stopping…", "");
  setTimeout(pollImp, 1200);
});

async function restoreImp() {
  const last = (await chrome.storage.local.get({ "tv-import-job": null }))["tv-import-job"];
  if (last) {
    if (last.running) {
      $("pull").hidden = true;
      $("stop").hidden = false;
      stopPolling();
      impPoll = setInterval(pollImp, 1000);
      impSet("pulling… " + impSummary(last), "");
    } else if (last.pulled || last.created || last.dupes || last.failed) {
      impSet(impSummary(last) + (last.aborted ? " — stopped" : " — done"), last.failed ? "bad" : "ok");
    }
  }
}

// ----- auto-sync controls -----
function autosyncStatusText(st, cfgSaved) {
  if (!st) return "";
  const every = Math.max(1, Number(st.intervalHours) || 6);
  let out = "off";
  if (st.enabled) out = `every ${every}h`;
  if (st.lastSync) {
    const when = new Date(st.lastSync);
    out += (st.enabled ? " · last " : " · last run ") + when.toLocaleString();
  }
  if (st.lastResult) out += " · " + st.lastResult;
  return out;
}

async function refreshAutoSync() {
  try {
    const r = await chrome.runtime.sendMessage({ type: "tv-autosync-get" });
    if (r && r.ok) {
      document.getElementById("autosync-status").textContent = autosyncStatusText(r.autoSync);
    }
  } catch (e) {}
}

async function saveAutoSyncFromUI() {
  const enabled = document.getElementById("autosync-on").checked;
  const intervalHours = Number(document.getElementById("autosync-every").value) || 6;
  const r = await chrome.runtime.sendMessage({
    type: "tv-autosync-save",
    autoSync: { enabled, intervalHours },
  });
  const el = document.getElementById("autosync-status");
  if (r && r.ok) el.textContent = autosyncStatusText(r.autoSync);
  else el.textContent = (r && r.error) || "failed to save";
}

document.getElementById("autosync-on").addEventListener("change", () => {
  saveAutoSyncFromUI().catch(() => {});
});
document.getElementById("autosync-every").addEventListener("change", () => {
  saveAutoSyncFromUI().catch(() => {});
});
document.getElementById("autosync-run").addEventListener("click", async () => {
  try {
    await saveAll();
    const r = await chrome.runtime.sendMessage({ type: "tv-autosync-run" });
    const el = document.getElementById("autosync-status");
    if (r && r.ok) {
      el.textContent = "running — watch the pull status above";
      stopPolling();
      impPoll = setInterval(pollImp, 1000);
      refreshAutoSync().catch(() => {});
    } else {
      el.textContent = (r && r.error) || "failed to start";
    }
  } catch (e) {
    document.getElementById("autosync-status").textContent = "cannot reach background";
  }
});

refreshAutoSync().catch(() => {});
