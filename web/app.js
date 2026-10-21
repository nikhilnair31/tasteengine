/* Taste Engine SPA — no deps. Auth'd fetch everywhere. */
"use strict";
try { const _k = "tv-debug-log"; const _a = JSON.parse(localStorage.getItem(_k) || "[]"); _a.push({ t: Date.now(), e: "script-top", href: location.href }); localStorage.setItem(_k, JSON.stringify(_a.slice(-50))); } catch (_) {}

const $ = (id) => document.getElementById(id);
const AUTH_KEY = "tv-auth";
const SERVER_KEY = "tv-server";

let state = {
  creds: null,
  filter: { kind: "", hue: "", source: "", tag: "" },
  sort: "random",
  offset: 0,
  query: "",
  mode: "auto",
  loading: false,
  hasMore: true,
  seed: Math.floor(Math.random() * 1e9),
};
const bulkSelected = new Set();
const BULK_MAX = 200;
let bulkRangeAnchor = null; // shift-click range start (card element)
let lastMeta = null;

function serverBase() {
  return localStorage.getItem(SERVER_KEY) || "";
}

function authHeader() {
  if (!state.creds) return {};
  return { Authorization: "Basic " + btoa(state.creds.user + ":" + state.creds.pass) };
}

async function api(path, opts = {}) {
  opts.headers = Object.assign(authHeader(), opts.headers || {});
  if (opts.body && !(opts.body instanceof FormData)) {
    opts.headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(opts.body);
  }
  const res = await fetch(serverBase() + path, opts);
  if (res.status === 401) {
    showLogin();
    throw new Error("auth required");
  }
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).detail || detail; } catch (e) {}
    throw new Error(detail);
  }
  return res.json();
}

/* ---------- media with auth → object URLs ---------- */
const blobCache = new Map();

const BLOB_CACHE_MAX = 240;
function mediaUrl(name) {
  if (!name) return "";
  if (name.startsWith("http")) return name;
  return serverBase() + "/media/" + encodeURIComponent(name);
}
function evictBlobCache() {
  while (blobCache.size > BLOB_CACHE_MAX) {
    const oldest = blobCache.keys().next().value;
    const url = blobCache.get(oldest);
    blobCache.delete(oldest);
    if (url) URL.revokeObjectURL(url);
  }
}
async function authedBlob(name) {
  if (blobCache.has(name)) {
    const hit = blobCache.get(name);
    blobCache.delete(name);
    blobCache.set(name, hit);
    return hit;
  }
  const res = await fetch(mediaUrl(name), { headers: authHeader() });
  if (!res.ok) throw new Error("media " + res.status);
  const url = URL.createObjectURL(await res.blob());
  blobCache.set(name, url);
  evictBlobCache();
  return url;
}
function clearBlobCache() {
  for (const u of blobCache.values()) URL.revokeObjectURL(u);
  blobCache.clear();
}

/* ---------- screens ---------- */
function showLogin() {
  $("main").classList.add("hidden");
  $("login").classList.remove("hidden");
}
function showMain() {
  $("login").classList.add("hidden");
  $("main").classList.remove("hidden");
  syncTopbarVar();
}

/* ---------- login ---------- */
$("login-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const user = $("login-user").value.trim();
  const pass = $("login-pass").value;
  state.creds = { user, pass };
  $("login-error").classList.add("hidden");
  try {
    const meta = await api("/api/meta");
    state.creds = { user, pass };
    localStorage.setItem(AUTH_KEY, JSON.stringify(state.creds));
    const settingsUser = $("settings-username");
    if (settingsUser) settingsUser.textContent = user;
    showMain();
    hydrateMeta(meta);
    await loadServerSettings();
    syncSortUI();
    await refresh({ replace: true });
    if (!new URLSearchParams(location.search).get("saved")) restoreView();
  } catch (err) {
    state.creds = null;
    $("login-error").textContent = "wrong credentials: " + err.message;
    $("login-error").classList.remove("hidden");
  }
});

/* ---------- quick note composer (inline card) ---------- */
const noteFiles = [];
let noteRecorder = null;
let noteRecordChunks = [];
let noteRecordStartedAt = 0;
let noteRecordTimer = null;
let noteRecordStopResolve = null;
let noteDiscardRecording = false;

const NOTE_MIC_ICON = '<svg viewBox="0 0 24 24"><rect x="9" y="2" width="6" height="13" rx="3"/><path d="M5 10a7 7 0 0 0 14 0"/><line x1="12" y1="17" x2="12" y2="22"/><line x1="8" y1="22" x2="16" y2="22"/></svg>';
const NOTE_ATTACH_ICON = '<svg viewBox="0 0 24 24"><path d="M12 16V4"/><polyline points="8 8 12 4 16 8"/><path d="M4 15v4a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-4"/></svg>';

function quickNoteHtml() {
  return `<div class="card card-note" data-note-composer="">
    <div class="note-tag">quick note</div>
    <textarea id="note-text" class="note-textarea" rows="1" placeholder="start typing here…" autocomplete="off" spellcheck="false"></textarea>
    <div id="note-attachments" class="note-attachments"></div>
    <div class="note-actions">
      <button id="note-attach" type="button" class="note-icon note-icon-attach" title="attach a file" aria-label="attach a file"><span class="ic">${NOTE_ATTACH_ICON}</span><span class="nl">attach</span></button>
      <input id="note-files" type="file" multiple accept="*/*" hidden>
      <button id="note-record" type="button" class="note-icon note-icon-attach" data-ic="voice" title="record a voice note" aria-label="record a voice note"><span class="ic">${NOTE_MIC_ICON}</span><span class="nl">voice</span></button>
      <span id="note-record-status" class="note-record-status" aria-live="polite"></span>
      <span class="note-hint">ctrl + enter to save</span>
    </div>
    <p id="note-status" class="import-status" aria-live="polite"></p>
  </div>`;
}

function installQuickNote() {
  const grid = $("grid");
  if (!grid || grid.querySelector(".card-note")) return;
  grid.insertAdjacentHTML("afterbegin", quickNoteHtml());
  wireQuickNote();
}

function wireQuickNote() {
  const text = $("note-text");
  const attach = $("note-attach");
  const files = $("note-files");
  const record = $("note-record");
  const card = document.querySelector(".card-note");
  if (!text || !files || !record || !card) return;
  if (attach) attach.addEventListener("click", () => files.click());
  files.addEventListener("change", (e) => { addNoteFiles(e.target.files); e.target.value = ""; });
  text.addEventListener("paste", (e) => {
    const pasted = [...(e.clipboardData?.items || [])].filter((item) => item.kind === "file").map((item) => item.getAsFile()).filter(Boolean);
    if (pasted.length) { e.preventDefault(); addNoteFiles(pasted); }
  });
  text.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); submitQuickNote(); return; }
  });
  text.addEventListener("input", () => {
    text.rows = Math.min(8, Math.max(1, text.value.split("\n").length));
    requestAnimationFrame(masonry);
  });
  record.addEventListener("click", () => {
    if (noteRecorder && noteRecorder.state === "recording") stopNoteRecording();
    else startNoteRecording();
  });
  card.addEventListener("dragover", (e) => { if (e.dataTransfer?.files?.length) e.preventDefault(); });
  card.addEventListener("drop", (e) => {
    if (!e.dataTransfer?.files?.length) return;
    e.preventDefault();
    addNoteFiles(e.dataTransfer.files);
  });
  card.addEventListener("click", (e) => {
    if (e.target.closest("button, input, textarea, a")) return;
    text.focus();
  });
}

function clearQuickNote() {
  if (noteRecorder && noteRecorder.state === "recording") {
    noteDiscardRecording = true;
    stopNoteRecording();
  }
  const text = $("note-text");
  if (text) { text.value = ""; text.rows = 1; }
  noteFiles.splice(0, noteFiles.length);
  const att = $("note-attachments");
  if (att) att.innerHTML = "";
  const st = $("note-status");
  if (st) { st.textContent = ""; st.className = "import-status"; }
  const rs = $("note-record-status");
  if (rs) rs.textContent = "";
}
function noteFileLabel(file) {
  const size = file.size < 1024 * 1024 ? `${Math.max(1, Math.round(file.size / 1024))} KB` : `${(file.size / 1024 / 1024).toFixed(1)} MB`;
  return `${file.name || "attachment"} · ${size}`;
}
function renderNoteAttachments() {
  const wrap = $("note-attachments");
  wrap.innerHTML = noteFiles.map((file, index) => `
    <div class="note-attachment">
      <span class="note-attachment-kind">${esc((file.type || "file").split("/")[0])}</span>
      <span class="note-attachment-name">${esc(noteFileLabel(file))}</span>
      <button type="button" class="note-attachment-remove" data-index="${index}" title="remove attachment">${ICONS.x}</button>
    </div>`).join("");
  wrap.querySelectorAll(".note-attachment-remove").forEach((button) => button.addEventListener("click", () => {
    noteFiles.splice(Number(button.dataset.index), 1);
    renderNoteAttachments();
  }));
}
function addNoteFiles(files) {
  for (const file of files || []) {
    if (!file || !file.size) continue;
    const duplicate = noteFiles.some((old) => old.name === file.name && old.size === file.size && old.lastModified === file.lastModified);
    if (!duplicate) noteFiles.push(file);
  }
  renderNoteAttachments();
}
function noteRecordTime() {
  const seconds = Math.floor((Date.now() - noteRecordStartedAt) / 1000);
  const mm = String(Math.floor(seconds / 60)).padStart(2, "0");
  const ss = String(seconds % 60).padStart(2, "0");
  return `${mm}:${ss}`;
}
function startNoteRecording() {
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia || !window.MediaRecorder) {
    $("note-record-status").textContent = "audio recording is not supported here";
    return;
  }
  navigator.mediaDevices.getUserMedia({ audio: true }).then((stream) => {
    noteRecordChunks = [];
    noteDiscardRecording = false;
    noteRecorder = new MediaRecorder(stream);
    noteRecordStartedAt = Date.now();
    noteRecorder.addEventListener("dataavailable", (event) => { if (event.data.size) noteRecordChunks.push(event.data); });
    noteRecorder.addEventListener("stop", () => {
      stream.getTracks().forEach((track) => track.stop());
      clearInterval(noteRecordTimer);
      const type = noteRecorder.mimeType || "audio/webm";
      const blob = new Blob(noteRecordChunks, { type });
      if (!noteDiscardRecording) {
        addNoteFiles([new File([blob], `note-recording-${new Date().toISOString().replace(/[:.]/g, "-")}.webm`, { type })]);
      }
      noteDiscardRecording = false;
      $("note-record").classList.remove("recording");
      $("note-record").classList.add("record-done");
      $("note-record").innerHTML = `<span class="ic">${NOTE_MIC_ICON}</span>`;
      $("note-record").title = "record a voice note";
      $("note-record-status").textContent = "recording attached";
      if (noteRecordStopResolve) {
        noteRecordStopResolve();
        noteRecordStopResolve = null;
      }
    });
    noteRecorder.start();
    $("note-record").classList.add("recording");
    $("note-record").classList.remove("record-done");
    $("note-record").innerHTML = `<span class="ic">${NOTE_MIC_ICON}</span>`;
    $("note-record").title = "stop recording";
    $("note-record-status").textContent = "00:00";
    noteRecordTimer = setInterval(() => { $("note-record-status").textContent = noteRecordTime(); }, 1000);
  }).catch(() => { $("note-record-status").textContent = "microphone access was denied"; });
}
function stopNoteRecording() {
  if (!noteRecorder || noteRecorder.state !== "recording") return Promise.resolve();
  return new Promise((resolve) => {
    noteRecordStopResolve = resolve;
    noteRecorder.stop();
  });
}
async function submitQuickNote() {
  if (noteRecorder && noteRecorder.state === "recording") await stopNoteRecording();
  const status = $("note-status");
  if (!status) return;
  const text = $("note-text");
  const content = text ? text.value : "";
  if (!content.trim() && !noteFiles.length) {
    status.className = "import-status error";
    status.textContent = "write something or attach a file";
    return;
  }
  status.className = "import-status";
  status.textContent = "saving note…";
  try {
    const body = new FormData();
    body.append("text", content);
    for (const file of noteFiles) body.append("files", file, file.name);
    const result = await api("/api/notes", { method: "POST", body });
    let fresh = null;
    try { fresh = await api("/api/items/" + result.id); } catch (fetchErr) {}
    clearQuickNote();
    status.className = "import-status success";
    status.textContent = "saved";
    try { hydrateMeta(await api("/api/meta")); } catch (metaErr) {}
    if (fresh) {
      const grid = $("grid");
      const composer = document.querySelector(".card-note");
      if (composer) composer.insertAdjacentHTML("afterend", cardHtml(fresh));
      else if (grid) grid.insertAdjacentHTML("afterbegin", cardHtml(fresh));
      masonry();
      await hydrateCards([fresh]);
      const freshCard = fresh && document.querySelector(`#grid .card[data-id="${fresh.id}"]`);
      if (freshCard) {
        freshCard.classList.add("is-opening");
        setTimeout(() => freshCard.classList.remove("is-opening"), 260);
      }
    }
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "save failed: " + err.message;
  }
}

/* ---------- follow-ups ---------- */
let fuOpen = false;
let fuItems = [];        // last fetched open follow-ups
let fuQuery = "";        // client-side search inside the follow-ups
let fuFilterKind = "";   // client-side kind filter inside the follow-ups
let fuStatus = "open";   // server-side status filter
const FU_KIND_FILTERS = ["listen", "watch", "app", "build", "games", "wishlist", "buy", "recipe", "visit"];

function renderFuStatusChips() {
  const wrap = $("chips-fu-status");
  if (!wrap) return;
  wrap.innerHTML = "";
  ["open", "done", "dismissed"].forEach((st) => {
    const b = document.createElement("button");
    b.className = "chip" + (fuStatus === st ? " active" : "");
    b.textContent = st;
    b.addEventListener("click", () => {
      fuStatus = st;
      renderFuStatusChips();
      loadFollowups();
    });
    wrap.appendChild(b);
  });
}

function closeFollowupsView() {
  closeRecipeReader();
  replaceViewFor("followups");
  fuOpen = false;
  $("main").classList.remove("followup-open");
  $("followups").classList.add("hidden");
  $("followups-list").innerHTML = "";
  fuQuery = "";
  fuFilterKind = "";
  $("filters").classList.remove("fu-mode");
  $("filters").classList.remove("trash-mode");
  $("search-q").placeholder = "search the vault…";
  syncViewRefreshSlot();
  syncViewPanelOffset();
  if (lastMeta && $("chips-kind")) renderChips("kind", lastMeta.kinds);
  syncGridSizeButton();
}
function openFollowupsView() {
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  closeDetail();
  closeInspireView();
  closeCleanupView(); // also collapses follow-ups (fuOpen=false) — harmless here
  closeTrashView();
  closeGroupsView();
  $("settings").classList.add("hidden");
  // Collapse the hover-expanded rail BEFORE measuring: full-page views render
  // at the collapsed 60px rail width, so the follow-ups do not open in a denser
  // layout just because the mouse is sitting over the rail.
  collapseHoverRail();
  fuOpen = true;
  const st = $("followups-status");
  st.className = "import-status";
  st.textContent = "";
  $("filters").classList.add("fu-mode");
  renderFuKindChips();
  renderFuStatusChips();
  $("search-q").placeholder = "search follow-ups…";
  setViewRefreshSlot(true);
  $("followups").classList.remove("hidden");
  $("main").classList.add("followup-open");
  loadFollowups();
  pushView("followups");
}
function isFollowupsOpen() { return fuOpen; }

/* ---------- active-view memory ----------
   The sidebar tabs are single-page navigation — clicking the tab you are
   already on stays put (no toggle back to posts). The current view is also
   written to the URL path (/posts, /cleanup, /trash, /settings, /followups,
   /inspire) so every page is reloadable and shareable; localStorage is the
   fallback for file:// or sandboxed contexts where history may fail. */
const VIEW_KEY = "te_view_v1";
const VIEW_PATHS = ["posts", "followups", "cleanup", "groups", "trash", "settings", "inspire"];
function currentViewPath() {
  try {
    const seg = location.pathname.split("/").filter(Boolean)[0];
    if (VIEW_PATHS.includes(seg)) return seg;
  } catch (e) {}
  return "posts";
}
/* openers push ONE history entry; closers only REPLACE the URL so nested
   close-inside-open calls never create junk intermediate entries.
   Both no-op the URL write while the login screen is up: the global
   outside-click handler fires during the login submit and its close-view
   calls would otherwise rewrite a deep-linked /settings (etc.) to /posts
   before restoreView() ever reads it. */
function routerReady() {
  const login = $("login");
  return !!(login && login.classList.contains("hidden"));
}
/* the deep-linked view, captured once at script load — the URL can be
   rewritten by stray handlers during the login/boot window, but this
   snapshot is what restoreView() trusts */
const INITIAL_VIEW = currentViewPath();
function pushView(v) {
  if (!routerReady()) { persistView(v); return; }
  try {
    const target = "/" + v;
    if (currentViewPath() !== v) history.pushState({ teView: v }, "", target);
    else history.replaceState({ teView: v }, "", target);
  } catch (e) {}
  persistView(v);
}
function replaceView(v) {
  if (!routerReady()) { persistView(v); return; }
  try { history.replaceState({ teView: v }, "", "/" + v); } catch (e) {}
  persistView(v);
}
/* closers use this: only rewrite the URL when the entry being left actually
   belongs to the view that is closing — a nested close-inside-open (e.g.
   openCleanupView also calling closeFollowupsView) must not rewrite the
   entry of the view the user deep-linked to */
function replaceViewFor(view) {
  if (currentViewPath() !== view) return;
  replaceView("posts");
}
function persistView(v) {
  try { localStorage.setItem(VIEW_KEY, v); } catch (e) {}
}
function viewFromSaved() {
  let v = "";
  try { v = localStorage.getItem(VIEW_KEY) || ""; } catch (e) {}
  return v;
}
function restoreView() {
  // URL wins over localStorage — the URL is what the user navigated to.
  let v = INITIAL_VIEW;
  if (v === "posts") {
    const stored = viewFromSaved();
    if (VIEW_PATHS.includes(stored)) v = stored;
  }
  if (v === "followups") openFollowupsView();
  else if (v === "trash") openTrashView();
  else if (v === "cleanup") openCleanupView();
  else if (v === "groups") openGroupsView();
  else if (v === "inspire") openInspireView();
  else if (v === "settings") { $("settings").classList.remove("hidden"); syncSettingsUI(); }
  else { v = "posts"; }
  try { history.replaceState({ teView: v }, "", "/" + v); } catch (e) {}
  return v;
}
window.addEventListener("popstate", () => {
  const v = currentViewPath();
  if (v === "posts") {
    if (isFollowupsOpen()) closeFollowupsView();
    if (isTrashOpen()) closeTrashView();
    if (isGroupsOpen()) closeGroupsView();
    if (!$("cleanup").classList.contains("hidden")) closeCleanupView();
    if (!$("settings").classList.contains("hidden")) { $("settings").classList.add("hidden"); }
    closeInspireView();
    persistView("posts");
    return;
  }
  const alreadyOpen = {
    followups: isFollowupsOpen(),
    trash: isTrashOpen(),
    groups: isGroupsOpen(),
    cleanup: !$("cleanup").classList.contains("hidden"),
    settings: !$("settings").classList.contains("hidden"),
    inspire: $("inspire") && !$("inspire").classList.contains("hidden"),
  }[v];
  if (!alreadyOpen) {
    if (v === "followups") openFollowupsView();
    else if (v === "trash") openTrashView();
    else if (v === "groups") openGroupsView();
    else if (v === "cleanup") openCleanupView();
    else if (v === "inspire") openInspireView();
    else if (v === "settings") { $("settings").classList.remove("hidden"); syncSettingsUI(); }
  }
});

function renderFuKindChips() {
  const wrap = $("chips-kind");
  if (!wrap) return;
  const all = document.createElement("button");
  all.className = "chip" + (fuFilterKind === "" ? " active" : "");
  all.textContent = "all";
  all.addEventListener("click", () => { fuFilterKind = ""; renderFuKindChips(); renderFollowups(); });
  wrap.innerHTML = "";
  wrap.appendChild(all);
  for (const kind of FU_KIND_FILTERS) {
    const n = fuItems.filter((f) => f.kind === kind).length;
    const btn = document.createElement("button");
    btn.className = "chip" + (fuFilterKind === kind ? " active" : "");
    btn.appendChild(document.createTextNode(kind));
    const cnt = document.createElement("span");
    cnt.className = "cnt";
    cnt.textContent = n;
    btn.appendChild(cnt);
    btn.addEventListener("click", () => {
      fuFilterKind = fuFilterKind === kind ? "" : kind;
      renderFuKindChips();
      renderFollowups();
    });
    wrap.appendChild(btn);
  }
}
$("btn-followups").addEventListener("click", (e) => {
  e.stopPropagation();
  if (!isFollowupsOpen()) openFollowupsView();
});
document.getElementById("btn-search-side")?.addEventListener("click", (e) => {
  e.stopPropagation();
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  $("settings").classList.add("hidden");
  closeDetail();
  closeCleanupView();
  closeTrashView();
  closeGroupsView();
  closeInspireView();
  if (isFollowupsOpen()) closeFollowupsView();
  $("search-q").focus();
  window.scrollTo(0, 0);
});
$("followups-refresh").addEventListener("click", loadFollowups);

const FU_KINDS = ["listen", "games", "app", "buy", "watch", "read", "recipe", "build", "visit", "todo"];

async function loadFollowups() {
  const list = $("followups-list");
  const status = $("followups-status");
  list.innerHTML = "";
  try {
    const data = await api("/api/followups?status=" + encodeURIComponent(fuStatus));
    if (data.error) throw new Error(data.error);
    fuItems = data.items || [];
    renderFollowups();
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "follow-ups failed: " + err.message;
  }
}

function renderFollowups() {
  const list = $("followups-list");
  const status = $("followups-status");
  let items = fuItems;
  const q = fuQuery.trim().toLowerCase();
  if (q) {
    items = items.filter((f) =>
      [f.title, f.why, f.kind, f.item_title].some((v) => String(v || "").toLowerCase().includes(q)));
  }
  if (fuFilterKind) items = items.filter((f) => f.kind === fuFilterKind);
  status.className = "import-status";
  if (!fuItems.length) {
    status.textContent = "";
    list.innerHTML = `<div class="fu-empty">nothing waiting — songs, shows, books, recipes, or games found in saved posts land here</div>`;
    return;
  }
  status.textContent = (q || fuFilterKind) && items.length !== fuItems.length
    ? `${items.length} of ${fuItems.length} follow-up${fuItems.length === 1 ? "" : "s"}${fuStatus === "open" ? " waiting" : " " + fuStatus}`
    : `${items.length} follow-up${items.length === 1 ? "" : "s"}${fuStatus === "open" ? " waiting" : " " + fuStatus}`;
  if (!items.length) {
    list.innerHTML = `<div class="fu-empty">no follow-ups match this filter.</div>`;
    return;
  }
  items = sortFollowups(items);
  list.innerHTML = items.map(fuCardHtml).join("");
  bindFuActions();
  if (isFollowupsOpen()) renderFuKindChips();
  fuMasonry();
}

function sortFollowups(items) {
  const arr = [...items];
  const s = state.sort;
  if (s === "alpha_asc") arr.sort((a, b) => String(a.title || "").localeCompare(String(b.title || "")));
  else if (s === "alpha_desc") arr.sort((a, b) => String(b.title || "").localeCompare(String(a.title || "")));
  else if (s === "oldest") arr.sort((a, b) => String(a.created_at || "").localeCompare(String(b.created_at || "")));
  else if (s === "random") {
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
  } else {
    arr.sort((a, b) => String(b.created_at || "").localeCompare(String(a.created_at || "")));
  }
  return arr;
}

function fuCardHtml(f) {
  const title = f.title || f.url || "?";
  const icon = FU_ICONS[f.kind] || FU_ICONS.todo;
  const kind = escAttr(f.kind);
  const titleHtml = f.url
    ? `<a class="fu-title" href="${escHtmlAttr(f.url)}" target="_blank" rel="noopener">${esc(title)} <span class="fu-open">↗</span></a>`
    : `<span class="fu-title">${esc(title)}</span>`;
  const storeKinds = ["amazon", "alibaba", "aliexpress", "temu", "seeed", "shopping"];
  const appHost = (f.url || "").toLowerCase();
  const appLabel = appHost.includes("apps.apple.com") || appHost.includes("itunes.apple.com")
    ? "app store"
    : appHost.includes("play.google.com")
      ? "google play"
      : "";
  // app-kind cards already say APP in the chip — no need for a duplicate store
  // sub-label. Keep it only for non-app links (amazon/alibaba/... etc.).
  const isAppKind = String(f.kind || "").toLowerCase() === "app";
  const storeKind = (!isAppKind && appLabel) || storeKinds.includes(String(f.kind || "").toLowerCase());
  const storeLabel = storeKind
    ? `<span class="fu-store-label">${esc(appLabel || (f.kind === "shopping" ? "store" : f.kind))}</span>`
    : "";
  const sourceThumb = storeKind && f.item_thumb
    ? `<img class="fu-product-thumb" data-src="${escHtmlAttr(f.item_thumb)}" alt="" loading="lazy">`
    : "";
  const why = f.why ? `<div class="fu-why">${esc(f.why)}</div>` : "";
  // Recipe cards are compact: they show a clamped preview and open the full
  // recipe in the reader overlay when clicked. The body is not dumped inline.
  const isRecipe = f.kind === "recipe";
  // Recipe cards show no ingredients preview — just the read-recipe link.
  const recipeBlock = "";
  const recipeHint = isRecipe
    ? `<a class="fu-open-recipe" data-recipe="${escAttr(f.id)}" title="read the full recipe"><span class="fu-ic-sm">${FU_ICONS.recipe}</span><span class="fu-open-label">read recipe</span></a>`
    : "";
  const src = f.item_title && f.item_title !== title
    ? `<button type="button" class="fu-src" data-item="${escAttr(f.item_id)}" title="open saved post">${esc(f.item_title)}</button>`
    : "";
  const wikiBtn = f.kind === "recipe" && f.body
    ? `<button type="button" class="ghost fu-wiki" data-fid="${escAttr(f.id)}" title="save this recipe to the wiki" aria-label="add recipe to the wiki"><svg viewBox="0 0 24 24" class="fu-btn-ic"><rect x="2" y="2" width="20" height="20"/><line x1="12" y1="6" x2="12" y2="18"/><line x1="6" y1="12" x2="18" y2="12"/></svg><span>add</span></button>`
    : "";
  return `<article class="fu-card" data-id="${escAttr(f.id)}" data-kind="${kind}"${isRecipe ? ` data-open="${escAttr(f.id)}"` : ""}>
    <div class="fu-head">
      <span class="fu-chip fu-kind-${kind}" title="${kind}">${icon}<span class="fu-kind-label">${esc(f.kind)}</span></span>
    </div>
    <div class="fu-main">${sourceThumb}${storeLabel}${titleHtml}${why}${recipeBlock}${recipeHint}${src}</div>
    <div class="fu-actions">
      ${wikiBtn}
      ${isRecipe ? "" : `<button type="button" class="ghost fu-done" title="mark done" aria-label="mark done"><svg viewBox="0 0 24 24" class="fu-btn-ic"><path d="M20 6L9 17l-5-5"/></svg><span>done</span></button>`}
      <button type="button" class="ghost fu-x" title="dismiss" aria-label="dismiss"><svg viewBox="0 0 24 24" class="fu-btn-ic"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg><span>dismiss</span></button>
    </div>
  </article>`;
}

const FU_ICONS = {
  listen: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>',
  games: '<svg viewBox="0 0 24 24" class="fu-ic"><line x1="6" y1="12" x2="10" y2="12"/><line x1="8" y1="10" x2="8" y2="14"/><line x1="15" y1="13" x2="15.01" y2="13"/><line x1="18" y1="11" x2="18.01" y2="11"/><rect x="2" y="6" width="20" height="12" rx="2"/></svg>',
  dev: '<svg viewBox="0 0 24 24" class="fu-ic"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>',
  buy: '<svg viewBox="0 0 24 24" class="fu-ic"><circle cx="9" cy="21" r="1"/><circle cx="20" cy="21" r="1"/><path d="M1 1h4l2.68 13.39a2 2 0 0 0 2 1.61h9.72a2 2 0 0 0 2-1.61L23 6H6"/></svg>',
  watch: '<svg viewBox="0 0 24 24" class="fu-ic"><polygon points="23 7 16 12 23 17 23 7"/><rect x="1" y="5" width="15" height="14" rx="2"/></svg>',
  read: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>',
  recipe: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M8 3h2M12 3h2M16 3h1"/><path d="M5 9h14M5 9l1 11h12l1-11"/><path d="M19 10h2a2 2 0 0 1 0 4h-2"/></svg>',
  app: '<svg viewBox="0 0 24 24" class="fu-ic"><rect x="3" y="3" width="8" height="8" rx="1"/><rect x="13" y="3" width="8" height="8" rx="1"/><rect x="3" y="13" width="8" height="8" rx="1"/><rect x="13" y="13" width="8" height="8" rx="1"/></svg>',
  build: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
  visit: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
  wishlist: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/></svg>',
  todo: '<svg viewBox="0 0 24 24" class="fu-ic"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>',
};

function bindFuActions() {
  document.querySelectorAll("#followups-list .fu-product-thumb[data-src]").forEach((im) => {
    authedBlob(im.dataset.src).then((url) => { im.src = url; }).catch(() => { im.remove(); });
  });
  document.querySelectorAll("#followups-list .fu-src").forEach((b) => {
    b.addEventListener("click", () => {
      const id = b.dataset.item;
      if (id) openDetail(id);
    });
  });
  document.querySelectorAll("#followups-list .fu-wiki").forEach((b) => {
    b.addEventListener("click", async () => {
      if (b.disabled) return;
      b.disabled = true;
      const label = b.querySelector("span");
      if (label) label.textContent = "saving…";
      try {
        await api(`/api/followups/${b.dataset.fid}/wiki`, { method: "POST", body: {} });
        // Saving to the wiki finishes the follow-up: auto-mark it done.
        setStatus(b, "done");
        if (isFollowupsOpen()) renderFuKindChips();
      } catch (err) {
        if (err.message !== "auth required") {
          if (label) label.textContent = "retry";
          b.disabled = false;
          console.error(err);
        }
      }
    });
  });
  const setStatus = async (btn, status) => {
    const row = btn.closest(".fu-card");
    if (!row) return;
    try {
      await api(`/api/followups/${row.dataset.id}`, { method: "POST", body: { status } });
      row.remove();
      fuItems = fuItems.filter((f) => f.id !== row.dataset.id);
      const statusEl = $("followups-status");
      const left = document.querySelectorAll("#followups-list .fu-card").length;
      statusEl.textContent = left ? `${left} follow-up${left === 1 ? "" : "s"} waiting` : "";
      if (!left) {
        $("followups-list").innerHTML = `<div class="fu-empty">nothing waiting — songs, shows, books, recipes, or games found in saved posts land here</div>`;
        $("followups-list").style.height = "";
      } else {
        fuMasonry();
      }
      if (isFollowupsOpen()) renderFuKindChips();
    } catch (err) {
      if (err.message !== "auth required") console.error(err);
    }
  };
  document.querySelectorAll("#followups-list .fu-done").forEach((b) => {
    b.addEventListener("click", () => setStatus(b, "done"));
  });
  document.querySelectorAll("#followups-list .fu-x").forEach((b) => {
    b.addEventListener("click", () => setStatus(b, "dismissed"));
  });
  // Clicking the recipe preview link (or the "read recipe" hint) opens the
  // reader with the raw text. The card body itself also opens it (not the
  // source post — the recipe IS the content here).
  document.querySelectorAll("#followups-list .fu-card[data-open]").forEach((card) => {
    card.addEventListener("click", (e) => {
      if (e.target.closest("button")) return;
      const recipeId = e.target.closest("[data-recipe]")?.dataset.recipe || card.dataset.open;
      if (recipeId) openRecipeReader(recipeId);
    });
  });
}

/* ---------- recipe reader (click a recipe follow-up) ---------- */
let recipeOpenFid = "";
function openRecipeReader(fid) {
  const f = fuItems.find((x) => x.id === fid);
  if (!f) return;
  recipeOpenFid = fid;
  $("rr-title").textContent = f.title || "recipe";
  const metaBits = [];
  if (f.item_title) metaBits.push(f.item_title);
  if (f.why) metaBits.push(f.why);
  $("rr-meta").textContent = metaBits.join(" · ");
  $("rr-body").textContent = f.body || "no recipe text on this one.";
  $("recipe-reader").classList.remove("hidden");
}
function closeRecipeReader() {
  $("recipe-reader").classList.add("hidden");
  recipeOpenFid = "";
}
$("rr-close").addEventListener("click", closeRecipeReader);
// The reader is read-only: no add/dismiss inside — those live on the card.
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("recipe-reader").classList.contains("hidden")) closeRecipeReader();
});

/* ---------- follow-ups masonry (mirrors the main grid) ---------- */
let fuMasonryTimer = 0;
function scheduleFuMasonry() {
  if (fuMasonryTimer) return;
  fuMasonryTimer = requestAnimationFrame(() => {
    fuMasonryTimer = 0;
    fuMasonry();
  });
}
function fuMasonry() {
  const list = $("followups-list");
  if (!list) return;
  const cards = [...list.querySelectorAll(".fu-card")];
  if (!cards.length) { list.style.height = ""; return; }
  // Same masonry as the main vault grid: card width follows the grid-size
  // setting (dense/regular/exploded), columns pack into the shortest column.
  // Honour the list padding so nothing bleeds past the right edge.
  const padL = parseFloat(getComputedStyle(list).paddingLeft) || 0;
  const padR = parseFloat(getComputedStyle(list).paddingRight) || 0;
  const padT = parseFloat(getComputedStyle(list).paddingTop) || 0;
  const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--grid-gap")) || 12;
  const width = list.clientWidth - padL - padR;
  const target = width <= 600 ? Math.min(settings.gridSize, 150) : settings.gridSize;
  const maxCols = width <= 600 ? 2 : 6;
  const cols = Math.max(1, Math.min(maxCols, Math.floor((width + gap) / (target + gap))));
  const colW = (width - gap * (cols - 1)) / cols;
  const tops = new Array(cols).fill(0);
  for (const card of cards) {
    card.style.width = colW + "px";
    let best = 0;
    for (let i = 1; i < cols; i++) if (tops[i] < tops[best]) best = i;
    card.style.left = (padL + best * (colW + gap)) + "px";
    card.style.top = (padT + tops[best]) + "px";
    tops[best] += card.offsetHeight + gap;
  }
  list.style.height = (padT + Math.max(...tops, 0)) + "px";
}


/* ---------- trash (soft-deleted posts) ---------- */
let trashOpen = false;
let trashItems = [];
let trashQuery = "";
let trashFilterKind = "";
let trashDays = 7;

/* Full-page views measure at the collapsed 60px rail width. If the mouse is
   still sitting over the rail when a view opens, the expanded 236px rail
   narrows the field and the view lays out denser than intended. Collapse it
   deliberately at open; the rail re-expands on hover afterwards. */
function collapseHoverRail() {
  document.body.classList.remove("rail-hover");
  try { syncTopbarVar(); } catch (e) {}
}

function closeTrashView() {
  trashOpen = false;
  $("main").classList.remove("trash-open");
  $("trash").classList.add("hidden");
  $("trash-list").innerHTML = "";
  trashItems = [];
  trashQuery = "";
  trashFilterKind = "";
  $("filters").classList.remove("trash-mode");
  $("filters").classList.remove("fu-mode");
  replaceViewFor("trash");
  $("search-q").placeholder = "search the vault…";
  syncViewRefreshSlot();
  syncViewPanelOffset();
  if (lastMeta && $("chips-kind")) renderChips("kind", lastMeta.kinds);
  syncGridSizeButton();
}
function openTrashView() {
  collapseHoverRail();
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  closeDetail();
  closeInspireView();
  closeCleanupView();
  closeFollowupsView(); // full-page views are exclusive
  closeGroupsView();
  $("settings").classList.add("hidden");
  trashOpen = true;
  $("trash").classList.remove("hidden");
  $("main").classList.add("trash-open");
  $("filters").classList.remove("fu-mode");
  $("filters").classList.add("trash-mode");
  renderTrashKindChips();
  $("search-q").placeholder = "search trash…";
  setViewRefreshSlot(true);
  loadTrash();
  pushView("trash");
}
function isTrashOpen() { return trashOpen; }

async function loadTrash() {
  const list = $("trash-list");
  const status = $("trash-status");
  // Same shimmer skeleton as the posts grid: lay out a card-shaped loading
  // grid instead of a "loading trash…" text line, so the view feels instant
  // while the /api/trash payload (up to hundreds of items) is in flight.
  showTrashLoadingSkeleton();
  try {
    const data = await api("/api/trash");
    const items = data.items || [];
    trashItems = items;
    trashDays = Number(data.trash_days || 7);
    status.className = "import-status";
    status.textContent = items.length
      ? `${items.length} trashed · auto-purge after ${trashDays} days`
      : "trash is empty";
    renderTrash();
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "trash failed: " + err.message;
    list.innerHTML = '<div class="trash-empty">trash failed to load.</div>';
  }
}

function showTrashLoadingSkeleton() {
  const list = $("trash-list");
  if (!list) return;
  list.innerHTML = Array.from({ length: 12 }, (_, i) => `
    <div class="card card-skeleton" aria-hidden="true" style="--skeleton-delay:${i * 35}ms">
      <div class="skeleton-media"></div>
      <div class="skeleton-body">
        <span class="skeleton-line skeleton-line-wide"></span>
        <span class="skeleton-line skeleton-line-short"></span>
      </div>
    </div>`).join("");
  trashMasonry();
}

function maybeRenderTrashEmpty() {
  if (trashItems.length) return;
  const list = $("trash-list");
  if (list) list.innerHTML = '<div class="trash-empty">nothing in the trash — deleted posts land here and purge automatically.</div>';
  const status = $("trash-status");
  if (status) { status.className = "import-status"; status.textContent = "trash is empty"; }
  renderTrashKindChips();
}

function sortTrash(items) {
  const arr = [...items];
  const s = state.sort;
  if (s === "alpha_asc") arr.sort((a, b) => String(a.title || "").localeCompare(String(b.title || "")));
  else if (s === "alpha_desc") arr.sort((a, b) => String(b.title || "").localeCompare(String(a.title || "")));
  else if (s === "oldest") arr.sort((a, b) => String(a.deleted_at || "").localeCompare(String(b.deleted_at || "")));
  else if (s === "frequent") { /* no click data for trashed items; keep deleted_at */ arr.sort((a, b) => String(b.deleted_at || "").localeCompare(String(a.deleted_at || ""))); }
  else if (s === "random") {
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
  } else {
    arr.sort((a, b) => String(b.deleted_at || "").localeCompare(String(a.deleted_at || "")));
  }
  return arr;
}

function renderTrash() {
  const list = $("trash-list");
  const status = $("trash-status");
  let items = trashItems;
  const q = trashQuery.trim().toLowerCase();
  if (q) {
    items = items.filter((it) =>
      [it.title, it.source, it.kind, it.text].some((v) => String(v || "").toLowerCase().includes(q)));
  }
  if (trashFilterKind) items = items.filter((it) => it.kind === trashFilterKind);
  items = sortTrash(items);
  if (!items.length) {
    if (q || trashFilterKind) {
      list.innerHTML = '<div class="trash-empty">no trashed posts match this filter.</div>';
    } else {
      list.innerHTML = '<div class="trash-empty">nothing in the trash — deleted posts land here and purge automatically.</div>';
    }
    status.className = "import-status";
    status.textContent = trashItems.length
      ? `${trashItems.length} trashed · auto-purge after ${trashDays} days`
      : "trash is empty";
    renderTrashKindChips();
    return;
  }
  status.className = "import-status";
  const filteredNote = (q || trashFilterKind) && items.length !== trashItems.length
    ? ` · showing ${items.length} of ${trashItems.length}`
    : "";
  status.textContent = `${items.length} trashed${filteredNote} · auto-purge after ${trashDays} days`;
  renderTrashChunked(list, items);
}

// Rendering 700+ trash cards in one innerHTML string + one masonry pass
// blocks the main thread for seconds. Paint the first page immediately, then
// append the rest in idle/rAF chunks.
let _trashChunkId = 0;
function renderTrashChunked(list, items) {
  const myId = ++_trashChunkId;
  const view = $("trash");
  const savedScroll = view ? view.scrollTop : 0;
  list.innerHTML = items.slice(0, 48).map(trashCardHtml).join("");
  bindTrashActions();
  renderTrashKindChips();
  trashMasonry(); // positions the first page + seeds _trashTops
  // A delete/restore mid-list must not yank the user back to the top:
  // restore the scroll position once the first chunk (and the region above
  // the current viewport) has painted and been laid out.
  if (view && savedScroll > 0) view.scrollTop = savedScroll;
  const append = (start) => {
    if (myId !== _trashChunkId || !isTrashOpen()) return;
    const chunk = items.slice(start, start + 120);
    list.insertAdjacentHTML("beforeend", chunk.map(trashCardHtml).join(""));
    bindTrashActions();
    // Position ONLY the freshly appended cards, continuing the column tops
    // saved by the last trashMasonry() call. Full re-masonry per chunk would
    // re-read 700+ cards every frame and re-stall the thread.
    trashMasonryAppend(start === 48 ? 48 : start, chunk.length, list);
    if (start + 120 < items.length) {
      requestAnimationFrame(() => append(start + 120));
    }
  };
  if (items.length > 48) requestAnimationFrame(() => append(48));
}

function trashMasonryAppend(skipCount, newCount, grid) {
  if (!grid) return;
  const firstNew = [...grid.querySelectorAll(".card")].slice(skipCount, skipCount + newCount);
  if (!firstNew.length) return;
  const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--grid-gap")) || 12;
  const padL = parseFloat(getComputedStyle(grid).paddingLeft) || 0;
  const padT = parseFloat(getComputedStyle(grid).paddingTop) || 0;
  const width = grid.clientWidth - padL - (parseFloat(getComputedStyle(grid).paddingRight) || 0);
  const target = width <= 600 ? Math.min(settings.gridSize, 150) : settings.gridSize;
  const maxCols = width <= 600 ? 2 : 6;
  const cols = Math.max(1, Math.min(maxCols, Math.floor((width + gap) / (target + gap))));
  const colW = (width - gap * (cols - 1)) / cols;
  const tops = (_trashTops && _trashTops.length === cols) ? [..._trashTops] : new Array(cols).fill(0);
  for (const card of firstNew) {
    card.style.width = colW + "px";
    let best = 0;
    for (let i = 1; i < cols; i++) if (tops[i] < tops[best]) best = i;
    card.style.left = (padL + best * (colW + gap)) + "px";
    card.style.top = (padT + tops[best]) + "px";
    tops[best] += card.offsetHeight + gap;
  }
  grid.style.height = (padT + Math.max(...tops, 0)) + "px";
  _trashTops = tops;
}

function renderTrashKindChips() {
  const wrap = $("chips-kind");
  if (!wrap || !$("filters").classList.contains("trash-mode")) return;
  const counts = {};
  for (const it of trashItems) counts[it.kind] = (counts[it.kind] || 0) + 1;
  const kinds = Object.keys(counts).sort();
  const all = document.createElement("button");
  all.className = "chip" + (trashFilterKind === "" ? " active" : "");
  all.textContent = "all";
  all.addEventListener("click", (e) => {
    e.stopPropagation();
    trashFilterKind = ""; renderTrashKindChips(); renderTrash();
  });
  wrap.innerHTML = "";
  wrap.appendChild(all);
  for (const kind of kinds) {
    const btn = document.createElement("button");
    btn.className = "chip" + (trashFilterKind === kind ? " active" : "");
    btn.appendChild(document.createTextNode(kind));
    const cnt = document.createElement("span");
    cnt.className = "cnt";
    cnt.textContent = counts[kind];
    btn.appendChild(cnt);
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      trashFilterKind = trashFilterKind === kind ? "" : kind;
      renderTrashKindChips();
      renderTrash();
    });
    wrap.appendChild(btn);
  }
}

function trashCardHtml(it) {
  // The exact same card as the posts tab, so clicking it opens the full
  // detail view. Only the hover actions differ: restore / delete now.
  const html = cardHtml(it);
  const marked = html.replace(/<div class="card/, "<div data-trash=\"1\" class=\"card\"");
  const trashActions = `<div class="card-hover-actions">
      <button type="button" class="card-hover-btn tr-hover-restore" data-card-id="${escAttr(it.id)}" title="restore this post to the vault" aria-label="restore this post">${ic("restore", "sm")}</button>
      <button type="button" class="card-hover-btn card-hover-delete tr-hover-destroy" data-card-id="${escAttr(it.id)}" title="permanently delete now — no undo" aria-label="permanently delete now">${ic("trash", "sm")}</button>
    </div>`;
  return marked.replace(/<div class="card-hover-actions">[\s\S]*?<\/div>/, trashActions);
}

function bindTrashActions() {
  // Lazy-load the same card media as the posts grid: observe placeholders so
  // an image only downloads as its card approaches the viewport. Eagerly
  // hydrating all ~700 trashed cards at once is what made the view stall.
  document.querySelectorAll("#trash-list .card-img.placeholder[data-media]").forEach((ph) => {
    if (!ph.dataset.loaded) observeCardLoad(ph);
  });
  // Removing one card must NOT re-render the whole list: a full re-render
  // (a) yanks the scroll back to the top, and (b) while the chunked rebuild
  // is in flight a re-click can land on another card's hover button under the
  // cursor → a second confirm, then "not found" once the id is gone. Remove
  // the card node in place and re-masonry incrementally instead.
  document.querySelectorAll("#trash-list .tr-hover-restore").forEach((b) => {
    b.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      try {
        await api(`/api/trash/${b.dataset.cardId}/restore`, { method: "POST", body: {} });
        _trashChunkId++; // cancel any in-flight chunked render
        trashItems = trashItems.filter((it) => it.id !== b.dataset.cardId);
        const card = b.closest(".card");
        if (card) card.remove();
        renderTrashKindChips();
        trashMasonry();
        refresh({ replace: true });
        maybeRenderTrashEmpty();
      } catch (err) {
        if (err.message !== "auth required") window.alert("restore failed: " + err.message);
      }
    });
  });
  document.querySelectorAll("#trash-list .tr-hover-destroy").forEach((b) => {
    b.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      // Pre-cancel chunked renders so the confirm + delete path never races
      // with a rebuild.
      if (!window.confirm("permanently delete this post? this cannot be undone.")) return;
      try {
        await api(`/api/trash/${b.dataset.cardId}/destroy`, { method: "POST", body: {} });
        _trashChunkId++; // cancel any in-flight chunked render
        trashItems = trashItems.filter((it) => it.id !== b.dataset.cardId);
        const card = b.closest(".card");
        if (card) card.remove();
        renderTrashKindChips();
        trashMasonry();
        maybeRenderTrashEmpty();
      } catch (err) {
        if (err.message !== "auth required") window.alert("delete failed: " + err.message);
      }
    });
  });
}

// Click a trash card -> the same detail view as the posts tab, but
// stripped: no refresh, no inspire (restore/delete live on the card hover).
// Bound ONCE at module scope — bindTrashActions() re-renders the list and
// must not re-add this listener every time (it used to stack duplicates).
$("trash-list").addEventListener("click", (e) => {
  const hoverBtn = e.target.closest(".card-hover-btn");
  const card = e.target.closest(".card");
  if (!card) return;
  if (hoverBtn) return; // handled above
  if (card.classList.contains("card-unavailable")) return;
  pulseCard(card);
  openDetail(card.dataset.id, { trash: true });
});

$("btn-trash").addEventListener("click", (e) => {
  e.stopPropagation();
  if (!isTrashOpen()) openTrashView();
});

/* ---------- cleanup similar posts ---------- */
let cleanupSel = {}; // itemId -> true (keep) / false (delete)
let cleanupGroups = [];
let cleanupFilter = ""; // evidence channel: "" | "source" | "text" | "media"
let cleanupScanned = 0;
let cleanupSort = "size_desc"; // sort key within the current filter
let cleanupQuery = ""; // text filter over the loaded groups

const CLEANUP_SORTS = [
  { value: "size_desc", label: "biggest groups first" },
  { value: "size_asc", label: "smallest groups first" },
  { value: "score_desc", label: "strongest match" },
  { value: "recent", label: "newest first" },
  { value: "oldest", label: "oldest first" },
];

function cleanupGroupSortKey(g, sort) {
  switch (sort) {
    case "size_desc": return -(g.full_size || 0);
    case "size_asc": return (g.full_size || 0);
    case "score_desc": return -(g.score || 0);
    case "recent": {
      let newest = "";
      for (const m of g.members || []) if (m.saved_at > newest) newest = m.saved_at;
      return newest;
    }
    case "oldest": {
      let oldest = "\uffff";
      for (const m of g.members || []) if (m.saved_at && m.saved_at < oldest) oldest = m.saved_at;
      return oldest === "\uffff" ? "" : oldest;
    }
    default: return 0;
  }
}

function applyCleanupSort(value) {
  cleanupSort = value;
  // Sort re-orders the SAME group set: preserve marks, just re-render.
  cleanupSel = {};
  // Keep the sort menu open after selecting (like the filter drawer); the
  // user closes it with the sort button or an outside click.
  syncCleanupSortUI();
  if (cleanupGroups.length) renderCleanupGroups(cleanupGroups);
}

function renderCleanupSortMenu() {
  const wrap = $("cleanup-sort-options");
  if (!wrap) return;
  wrap.innerHTML = "";
  for (const opt of CLEANUP_SORTS) {
    const btn = document.createElement("button");
    btn.className = "sort-option" + (cleanupSort === opt.value ? " active" : "");
    btn.dataset.sort = opt.value;
    btn.textContent = opt.label;
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      applyCleanupSort(opt.value);
    });
    wrap.appendChild(btn);
  }
  syncCleanupSortUI();
}

function syncCleanupSortUI() {
  document.querySelectorAll("#cleanup-sort-options .sort-option").forEach((option) => {
    option.classList.toggle("active", option.dataset.sort === cleanupSort);
  });
  const label = CLEANUP_SORTS.find((s) => s.value === cleanupSort)?.label || "";
  // The shared toolbar sort button shows the active cleanup sort while the
  // cleanup view is open; the standalone button (mobile) also gets the title.
  const mainBtn = $("btn-sort");
  if (mainBtn && $("main").classList.contains("cleanup-open")) {
    mainBtn.title = label ? "sort: " + label : "sort";
  }
  const btn = $("btn-cleanup-sort");
  if (btn) btn.title = label ? "sort: " + label : "sort cleanup groups";
}

function setCleanupFilterBtnUI(open) {
  // The shared #btn-filters icon tracks the cleanup pane while cleanup is
  // open. Never touch it otherwise (it belongs to the vault #filters then).
  if (!$("main").classList.contains("cleanup-open")) return;
  const fic = $("filters-ic");
  if (fic) fic.innerHTML = open ? (ICONS.x || fic.innerHTML) : (ICONS.filters || fic.innerHTML);
  const fbtn = $("btn-filters");
  if (fbtn) fbtn.title = open ? "close cleanup filters" : "filter cleanup groups";
}

// The search-box slot swaps one shared button: #btn-inspire in the vault,
// #btn-view-refresh in any full-page view (cleanup / follow-ups / trash).
function setViewRefreshSlot(showRefresh) {
  const insp = $("btn-inspire");
  if (insp) insp.classList.toggle("hidden", !!showRefresh);
  const refr = $("btn-view-refresh");
  if (refr) refr.classList.toggle("hidden", !showRefresh);
}

// The slot must ALWAYS match the open full-page view. Deriving it from the
// live view state (instead of hard-coding on each open/close) kills the whole
// class of bugs where one view's close() wiped another view's refresh button.
function syncViewRefreshSlot() {
  const main = $("main");
  const open = isTrashOpen() || isFollowupsOpen() || isGroupsOpen() ||
    !!(main && main.classList.contains("cleanup-open"));
  setViewRefreshSlot(open);
}

// The refresh button in the toolbar slot dispatches to whichever view is open.
function runViewRefresh() {
  if ($("main").classList.contains("cleanup-open")) { runCleanupRefresh(); return; }
  if (isGroupsOpen()) { loadGroups(); return; }
  if (isFollowupsOpen()) { loadFollowups(); return; }
  if (isTrashOpen()) { loadTrash(); return; }
}

function openCleanupFilterMenu() {
  const m = $("cleanup-filter-menu");
  if (!m) return;
  closeCleanupSortMenu();
  m.classList.remove("hidden");
  renderCleanupFilterChips();
  setCleanupFilterBtnUI(true);
  syncViewPanelOffset();
}

function closeCleanupFilterMenu() {
  const m = $("cleanup-filter-menu");
  if (m) m.classList.add("hidden");
  setCleanupFilterBtnUI(false);
  syncViewPanelOffset();
}

function closeCleanupSortMenu() {
  const m = $("cleanup-sort-menu");
  if (m) m.classList.add("hidden");
  syncViewPanelOffset();
}

// Full-page views lay out the card grid against .trash-list / .followups-list
// / .cleanup-groups. The filter/sort drawers float OVER those views (they
// live outside the view element), so opening a drawer would cover the first
// card row — with hover X/delete buttons peeking around the drawer edges.
// This marks #main so CSS can push the view content down under the drawer.
function syncViewPanelOffset() {
  const main = $("main");
  if (!main) return;
  const fOpen = !$("filters").classList.contains("collapsed");
  const sOpen = !$("sort-menu").classList.contains("hidden");
  const cfOpen = $("cleanup-filter-menu") && !$("cleanup-filter-menu").classList.contains("hidden");
  const csOpen = $("cleanup-sort-menu") && !$("cleanup-sort-menu").classList.contains("hidden");
  main.classList.toggle("view-filter-open", fOpen && (isTrashOpen() || isFollowupsOpen()));
  main.classList.toggle("view-sort-open", sOpen && (isTrashOpen() || isFollowupsOpen()));
  main.classList.toggle("cleanup-panel-open", (cfOpen || csOpen) && $("main").classList.contains("cleanup-open"));
}

const CLEANUP_EVIDENCE = [
  { value: "source", label: "same link" },
  { value: "media", label: "same image" },
  { value: "empty", label: "empty link" },
];

function cleanupGroupMatches(g) {
  const q = cleanupQuery.trim().toLowerCase();
  if (!q) return true;
  const hay = [
    g.leader || "",
    evidenceLabel(g),
    ...(g.members || []).flatMap((m) => [m.title, m.url, m.source, m.kind]),
  ].filter(Boolean).join(" ").toLowerCase();
  return hay.includes(q);
}

function cleanupStatusText(matchedCount, visibleCount, totalCount, scanned) {
  const scannedTxt = Number(scanned || 0).toLocaleString();
  let cluster = "";
  if (cleanupFilter) {
    const label = CLEANUP_EVIDENCE.find((e) => e.value === cleanupFilter)?.label || cleanupFilter;
    cluster = `${visibleCount} of ${totalCount} ${label} groups`;
  } else {
    cluster = `${totalCount} groups of similar posts`;
  }
  if (cleanupQuery.trim()) {
    cluster = `${matchedCount} of ${visibleCount} matching “${cleanupQuery.trim()}”`;
  }
  return `${cluster} · ${scannedTxt} scanned · click opens · ctrl+click marks keep/delete`;
}

function applyCleanupFilter(value) {
  cleanupFilter = value;
  cleanupSel = {};
  const bar = $("cleanup-bulk-bar");
  if (bar) bar.classList.add("hidden");
  const del = $("cleanup-delete");
  if (del) del.setAttribute("data-state", "idle");
  closeCleanupSortMenu();
  renderCleanupFilterChips();
  syncCleanupTypeDeleteBtn();
  if (cleanupGroups.length) renderCleanupGroups(cleanupGroups);
}

function renderCleanupFilterChips() {
  const wrap = $("chips-cleanup-evidence");
  if (!wrap) return;
  const cur = cleanupFilter;
  const all = document.createElement("button");
  all.className = "chip" + (cur === "" ? " active" : "");
  all.textContent = "all";
  all.addEventListener("click", (e) => {
    // The re-render detaches this button before the click bubbles; without
    // stopPropagation the document outside-click handler sees a detached
    // target (closest('#cleanup') === null) and closes the whole view.
    e.stopPropagation();
    if (cleanupFilter !== "") applyCleanupFilter("");
  });
  wrap.innerHTML = "";
  wrap.appendChild(all);
  for (const ev of CLEANUP_EVIDENCE) {
    const count = cleanupGroups.filter((g) => (g.evidence || []).includes(ev.value)).length;
    const btn = document.createElement("button");
    btn.className = "chip" + (cur === ev.value ? " active" : "");
    const lbl = document.createElement("span");
    lbl.textContent = ev.label;
    btn.appendChild(lbl);
    const cnt = document.createElement("span");
    cnt.className = "cnt";
    cnt.textContent = count;
    btn.appendChild(cnt);
    btn.addEventListener("click", (e) => {
      // Same detachment issue as the "all" chip: keep the click from
      // bubbling to the document outside-click handler.
      e.stopPropagation();
      applyCleanupFilter(cur === ev.value ? "" : ev.value);
    });
    wrap.appendChild(btn);
  }
}

function closeCleanupView() {
  replaceViewFor("cleanup");
  $("main").classList.remove("cleanup-open");
  $("cleanup").classList.add("hidden");
  $("cleanup-groups").innerHTML = "";
  cleanupGroups = [];
  cleanupSel = {};
  cleanupFilter = "";
  cleanupScanned = 0;
  disconnectCleanupObserver();
  // Restore the search-box slot: only a full-page view keeps the refresh
  // button; derive from what is actually open (2026-08-26: an unconditional
  // reset here made cleanup's row-refresh revert the slot to inspire).
  syncViewRefreshSlot();
  // Reset the shared filter button icon/title back to the vault filters.
  const fic = $("filters-ic");
  if (fic) fic.innerHTML = ICONS && ICONS.filters ? ICONS.filters : fic.innerHTML;
  const fbtn = $("btn-filters");
  if (fbtn) fbtn.title = "filters";
  $("search-q").placeholder = "search the vault…";
  cleanupQuery = "";
  syncSortUI(); // sort button title returns to the vault sort
  const bar = $("cleanup-bulk-bar");
  if (bar) bar.classList.add("hidden");
  const count = $("cleanup-bulk-count");
  if (count) count.textContent = "0";
  const del = $("cleanup-delete");
  if (del) del.setAttribute("data-state", "idle");
  closeCleanupFilterMenu();
  closeCleanupSortMenu();
  syncCleanupTypeDeleteBtn();
  syncViewPanelOffset();
}
function openCleanupView() {
  collapseHoverRail();
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  closeDetail();
  closeInspireView();
  closeFollowupsView(); // full-page views are exclusive
  closeTrashView();
  closeGroupsView();
  $("settings").classList.add("hidden");
  $("cleanup").classList.remove("hidden");
  $("main").classList.add("cleanup-open");
  // From the search box: hide inspire, show the view refresh in its slot.
  setViewRefreshSlot(true);
  closeCleanupFilterMenu();
  closeCleanupSortMenu();
  renderCleanupSortMenu();
  syncGridSizeButton();
  $("search-q").placeholder = "search groups…";
  runCleanupScan();
  pushView("cleanup");
}
$("btn-cleanup").addEventListener("click", (e) => {
  e.stopPropagation();
  const view = $("cleanup");
  if (view.classList.contains("hidden")) openCleanupView();
});
function runCleanupRefresh() {
  cleanupSel = {};
  const bar = $("cleanup-bulk-bar");
  if (bar) bar.classList.add("hidden");
  runCleanupScan();
}
$("btn-view-refresh").addEventListener("click", runViewRefresh);
$("btn-cleanup-filters").addEventListener("click", (e) => {
  e.stopPropagation();
  const menu = $("cleanup-filter-menu");
  closeCleanupSortMenu();
  if (menu.classList.contains("hidden")) {
    menu.classList.remove("hidden");
    renderCleanupFilterChips();
  } else {
    menu.classList.add("hidden");
  }
});
$("btn-cleanup-sort").addEventListener("click", (e) => {
  e.stopPropagation();
  const menu = $("cleanup-sort-menu");
  closeCleanupFilterMenu();
  renderCleanupSortMenu();
  menu.classList.toggle("hidden");
});

async function runCleanupScan() {
  const status = $("cleanup-status");
  $("cleanup-bulk-bar").classList.add("hidden");
  const td = $("cleanup-type-delete");
  if (td) td.classList.add("hidden");
  status.className = "import-status";
  status.textContent = "scanning for similar posts…";
  $("cleanup-groups").innerHTML = "";
  cleanupGroups = [];
  cleanupSel = {};
  try {
    const data = await api("/api/cleanup/groups");
    if (data.error) throw new Error(data.error);
    status.className = "import-status";
    cleanupScanned = Number(data.scanned || 0);
    if (!data.groups || !data.groups.length) {
      status.textContent = "no similar posts found across " + cleanupScanned.toLocaleString() + " saved";
      cleanupGroups = [];
      renderCleanupFilterChips();
      syncCleanupTypeDeleteBtn();
      return;
    }
    cleanupGroups = data.groups;
    renderCleanupFilterChips();
    renderCleanupSortMenu();
    renderCleanupGroups(data.groups);
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "cleanup scan failed: " + err.message;
  }
}

let cleanupRangeAnchor = null; // {group, card} for shift-click range select

function applyCleanupCardState(card, keep) {
  const id = card.dataset.id;
  cleanupSel[id] = keep;
  card.classList.toggle("is-delete", !keep);
  card.querySelector(".cleanup-kbd").textContent = keep ? "keep" : "delete";
}

function renderCleanupGroups(groups) {
  cleanupGroups = groups;
  cleanupRangeAnchor = null;
  const visible = cleanupFilter
    ? groups.filter((g) => (g.evidence || []).includes(cleanupFilter))
    : groups;
  const matched = cleanupQuery.trim() ? visible.filter(cleanupGroupMatches) : visible;
  const status = $("cleanup-status");
  status.className = "import-status";
  status.textContent = cleanupStatusText(matched.length, visible.length, groups.length, cleanupScanned);
  const sorted = [...matched].sort((a, b) => {
    const ka = cleanupGroupSortKey(a, cleanupSort);
    const kb = cleanupGroupSortKey(b, cleanupSort);
    if (typeof ka === "number" && typeof kb === "number") return ka - kb;
    return String(ka).localeCompare(String(kb));
  });
  // Default: every member is KEEP. The suggested keeper (leader) gets a
  // hint dot; nothing is pre-checked for deletion — the user taps the
  // redundant copies themselves. `delete N` only counts what they mark.
  const wrap = $("cleanup-groups");
  wrap.innerHTML = sorted.map((group, gi) => {
    const ev = evidenceLabel(group);
    const extra = group.full_size > group.members.length ? ` · +${group.full_size - group.members.length} more` : "";
    // Empty-link groups are junk saves, not similar posts.
    const headLabel = (group.evidence || []).includes("empty")
      ? `${group.members.length} empty links${extra}`
      : `${group.members.length} similar${extra}`;
    const members = group.members.map((m) => {
      cleanupSel[m.id] = true;
      return cleanupCardHtml(m, gi, m.id === group.leader, group.members.length, ev + extra);
    }).join("");
    return `<section class="cleanup-group" data-group="${gi}" data-gsize="${group.members.length}">
      <div class="cleanup-group-head">
        <span class="cleanup-group-title">${headLabel}</span>
        <span class="cleanup-evidence">${ev}${group.score ? ` · ${Math.round(group.score * 100)}%` : ""}</span>
      </div>
      <div class="cleanup-cards" data-gsize="${group.members.length}">${members}</div>
    </section>`;
  }).join("");
  wrap.querySelectorAll(".cleanup-card").forEach((card) => {
    card.addEventListener("click", (e) => {
      const id = card.dataset.id;
      // Ctrl/Cmd+click toggles one card and sets it as the range anchor.
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        applyCleanupCardState(card, !cleanupSel[id]);
        cleanupRangeAnchor = { group: card.closest(".cleanup-cards"), card };
        syncCleanupDeleteBar();
        return;
      }
      // Shift+click selects every card between the anchor and this card in
      // the same group (all marked keep if the anchor was kept, else delete).
      if (e.shiftKey && cleanupRangeAnchor) {
        e.preventDefault();
        const group = card.closest(".cleanup-cards");
        if (group && group === cleanupRangeAnchor.group) {
          const cards = [...group.querySelectorAll(".cleanup-card")];
          const a = cards.indexOf(cleanupRangeAnchor.card);
          const b = cards.indexOf(card);
          if (a !== -1 && b !== -1) {
            const [lo, hi] = a < b ? [a, b] : [b, a];
            // The range takes the anchor's keep/deleted state.
            const keep = !!cleanupSel[cleanupRangeAnchor.card.dataset.id];
            for (let i = lo; i <= hi; i++) applyCleanupCardState(cards[i], keep);
            cleanupRangeAnchor = { group, card };
            syncCleanupDeleteBar();
            return;
          }
        }
      }
      cleanupRangeAnchor = null;
      openDetail(id);
    });
  });
  hydrateCleanupMedia(wrap);
  syncCleanupDeleteBar();
  syncCleanupTypeDeleteBtn();
  applyCleanupGridSize();
  cleanupMasonry();
}

function evidenceLabel(group) {
  const map = { source: "same link", media: "same image", empty: "empty link" };
  const labels = (group.evidence || []).map((e) => map[e] || e);
  return labels.length ? labels.join(" + ") : "similar";
}

function cleanupCardHtml(m, gi, isLeader, groupSize, evLabel) {
  const title = displayTitle(m) || m.source || "saved post";
  const keep = cleanupSel[m.id];
  const ar = coverAR(m);
  let inner;
  if (m.cover && m.cover.thumb) {
    inner = `<div class="cleanup-ph" data-ph="${esc(m.cover.thumb)}" data-id="${esc(m.id)}" style="aspect-ratio:${ar}"><span class="thumb-skeleton" aria-hidden="true"></span></div>`;
  } else {
    inner = `<div class="cleanup-ph cleanup-ph-text" style="aspect-ratio:${ar};background:${coverColor(m)}22;color:${coverColor(m)}">${esc(title.slice(0, 90))}</div>`;
  }
  // Leader marker: a small accent dot (the accent border already signals the
  // suggested keeper). No extra "keep" text — the foot pill says it once.
  const dot = isLeader ? `<span class="cleanup-leader" title="suggested keeper">suggested</span>` : "";
  const verifyBadge = isLeader
    ? `<span class="cleanup-suggested" title="the keeper this set was built around">suggested</span>`
    : "";
  return `<article class="cleanup-card${keep ? "" : " is-delete"}${isLeader ? " is-leader" : ""}" data-id="${esc(m.id)}" data-group="${gi}" data-ar="${ar}" title="${esc(m.url || title)}">
    ${inner}
    ${dot}
    ${verifyBadge}
    <div class="cleanup-card-foot">
      <span class="cleanup-kbd">${keep ? "keep" : "delete"}</span>
      <span class="cleanup-card-title">${esc(title.slice(0, 44))}</span>
    </div>
  </article>`;
}

let cleanupCardObserver = null;

function observeCleanupMedia(ph) {
  if (!cleanupCardObserver) {
    cleanupCardObserver = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const el = entry.target;
        if (el.dataset.loaded) continue;
        el.dataset.loaded = "1";
        cleanupCardObserver.unobserve(el);
        hydrateCleanupPh(el);
      }
    }, { rootMargin: "300px" });
  }
  cleanupCardObserver.observe(ph);
}

function hydrateCleanupPh(ph) {
  authedBlob(ph.dataset.ph).then((url) => {
    if (!ph.isConnected) return;
    const im = document.createElement("img");
    im.className = "cleanup-img";
    im.style.aspectRatio = ph.style.aspectRatio || "1";
    im.loading = "eager";
    im.decoding = "async";
    const reveal = () => {
      if (!ph.isConnected) return;
      ph.replaceWith(im);
    };
    im.addEventListener("load", reveal, { once: true });
    im.addEventListener("error", () => {
      if (!ph.isConnected) return;
      ph.classList.add("media-unavailable");
      ph.innerHTML = '<span class="media-status-mark">×</span>';
    }, { once: true });
    im.src = url;
    if (im.complete && im.naturalWidth) reveal();
  }).catch(() => {
    if (!ph.isConnected) return;
    ph.classList.add("media-unavailable");
    ph.innerHTML = '<span class="media-status-mark">×</span>';
  });
}

function hydrateCleanupMedia(root) {
  if (!root) return;
  // Lazy: a cleanup review can hold hundreds of groups (~1,000+ member
  // thumbs). Observe so blobs download as groups approach the viewport,
  // never all at once on open.
  root.querySelectorAll(".cleanup-ph[data-ph]").forEach((ph) => {
    if (ph.dataset.loaded) return;
    observeCleanupMedia(ph);
  });
}

function disconnectCleanupObserver() {
  if (cleanupCardObserver) {
    cleanupCardObserver.disconnect();
    cleanupCardObserver = null;
  }
}

function syncCleanupDeleteBar() {
  const delIds = Object.entries(cleanupSel).filter(([, keep]) => !keep).map(([id]) => id);
  const bar = $("cleanup-bulk-bar");
  const count = $("cleanup-bulk-count");
  const del = $("cleanup-delete");
  if (!delIds.length) {
    bar.classList.add("hidden");
    del.setAttribute("data-state", "idle");
    return;
  }
  bar.classList.remove("hidden");
  count.textContent = delIds.length;
  del.title = `delete ${delIds.length} post${delIds.length === 1 ? "" : "s"}`;
}

$("cleanup-delete").addEventListener("click", async (e) => {
  const del = $("cleanup-delete");
  const bar = $("cleanup-bulk-bar");
  const count = $("cleanup-bulk-count");
  const delIds = Object.entries(cleanupSel).filter(([, keep]) => !keep).map(([id]) => id);
  if (!delIds.length) return;
  if (!window.confirm(`delete ${delIds.length} post${delIds.length === 1 ? "" : "s"}?`)) return;
  const status = $("cleanup-status");
  del.disabled = true;
  del.title = "deleting…";
  status.className = "import-status";
  status.textContent = `deleting ${delIds.length} posts…`;
  try {
    const result = await api("/api/cleanup/delete", { method: "POST", body: { ids: delIds } });
    status.className = "import-status success";
    status.textContent = `deleted ${Number(result.deleted || 0).toLocaleString()} · ${Number(result.missing || 0).toLocaleString()} missing`;
    // Rebuild selection state and rescan.
    cleanupSel = {};
    cleanupGroups = [];
    del.setAttribute("data-state", "idle");
    bar.classList.add("hidden");
    if (count) count.textContent = "0";
    await runCleanupScan();
    const meta = await api("/api/meta");
    hydrateMeta(meta);
    state.offset = 0;
    state.seed = Math.floor(Math.random() * 1e9);
    await refresh({ replace: true });
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "delete failed: " + err.message;
    del.setAttribute("data-state", "idle");
    syncCleanupDeleteBar();
  } finally {
    del.disabled = false;
  }
});

/* ---- one-click: delete every non-suggested member (active type or ALL) ---- */
// Count = non-leader members. With an evidence filter active, only groups of
// that evidence count; with no filter ("all"), every group counts — the same
// set the backend /api/cleanup/delete-type resolves for channel "all".
function cleanupTypeDeleteCount() {
  let n = 0;
  for (const g of cleanupGroups) {
    if (cleanupFilter && !(g.evidence || []).includes(cleanupFilter)) continue;
    for (const m of g.members || []) if (m.id !== g.leader) n++;
  }
  return n;
}

function cleanupTypeDeleteLabel() {
  if (!cleanupFilter) return "";
  return CLEANUP_EVIDENCE.find((e) => e.value === cleanupFilter)?.label || cleanupFilter;
}

function syncCleanupTypeDeleteBtn() {
  const btn = $("cleanup-type-delete");
  if (!btn) return;
  const n = cleanupTypeDeleteCount();
  if (!n) { btn.classList.add("hidden"); return; }
  btn.classList.remove("hidden");
  const label = cleanupTypeDeleteLabel();
  // Empty-link groups have NO suggested keeper (leader is None): every
  // member is a delete candidate, so phrase the action as a plain sweep,
  // not "keep the suggested keeper".
  const title = label === "empty link"
    ? `delete ${n} empty link posts (no URL saved)`
    : label
      ? `keep the suggested keeper, delete ${n} other ${label} posts`
      : `keep the suggested keeper, delete ${n} other posts`;
  btn.title = title;
  btn.setAttribute("aria-label", title);
}

$("cleanup-type-delete").addEventListener("click", async (e) => {
  e.stopPropagation();
  const btn = $("cleanup-type-delete");
  const n = cleanupTypeDeleteCount();
  if (!n) { btn.classList.add("hidden"); return; }
  const label = cleanupTypeDeleteLabel();
  const scope = label ? `${label} ` : "";
  let msg;
  if (label === "empty link") {
    msg = `delete ${n} empty link posts? No URL was saved for them.`;
  } else if (label) {
    msg = `delete ${n} non-suggested ${label} posts? The suggested keeper in each group stays.`;
  } else {
    msg = `delete ${n} non-suggested posts? The suggested keeper in each group stays.`;
  }
  if (!window.confirm(msg)) return;
  btn.disabled = true;
  const status = $("cleanup-status");
  status.className = "import-status";
  status.textContent = `deleting ${n} ${scope}posts…`;
  try {
    const result = await api("/api/cleanup/delete-type", { method: "POST", body: { channel: cleanupFilter || "all" } });
    status.className = "import-status success";
    status.textContent = `deleted ${Number(result.deleted || 0).toLocaleString()} · ${Number(result.missing || 0).toLocaleString()} missing`;
    cleanupSel = {};
    cleanupGroups = [];
    btn.classList.add("hidden");
    const bar = $("cleanup-bulk-bar");
    if (bar) bar.classList.add("hidden");
    const count = $("cleanup-bulk-count");
    if (count) count.textContent = "0";
    const del = $("cleanup-delete");
    if (del) del.setAttribute("data-state", "idle");
    await runCleanupScan();
    const meta = await api("/api/meta");
    hydrateMeta(meta);
    state.offset = 0;
    state.seed = Math.floor(Math.random() * 1e9);
    await refresh({ replace: true });
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "delete failed: " + err.message;
  } finally {
    btn.disabled = false;
  }
});

/* ---------- groups (similarity clusters, server-side k-means) ----------
   Tiles render the cluster list; a tile click (or the shuffle button) swaps
   to the group detail: the same .card component as the vault grid, so card
   clicks reuse the shared openDetail(). Cover thumbs resolve each covers[]
   item id through /api/items/{id} (cached), then lazy-load like any card. */
let groupsOpen = false;
let groupsCluster = null;     // the one cluster on screen
let groupsItems = [];         // its cards
let groupsMatched = -1;       // how many pass the current filter (-1 = unknown)
const groupsSeen = new Set(); // cluster ids already shown — sent to /random as exclude
let groupsFilter = { kind: "", source: "", hue: "", tag: "" };

function isGroupsOpen() { return groupsOpen; }

function closeGroupsView() {
  replaceViewFor("groups");
  $("main").classList.remove("groups-open");
  $("groups").classList.add("hidden");
  $("groups-items").innerHTML = "";
  groupsCluster = null;
  groupsItems = [];
  groupsMatched = -1;
  groupsFilter = { kind: "", source: "", hue: "", tag: "" };
  $("filters").classList.remove("groups-mode");
  $("search-q").placeholder = "search the vault…";
  syncViewRefreshSlot();
  syncViewPanelOffset();
  // our rows replaced the vault's, so put all four back
  if (lastMeta) {
    for (const group of ["kind", "source", "hue", "tag"]) {
      if ($("chips-" + group)) renderChips(group, lastMeta[group + "s"]);
    }
  }
  syncGridSizeButton();
}

function openGroupsView() {
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  closeDetail();
  closeInspireView();
  closeCleanupView();
  closeFollowupsView(); // full-page views are exclusive
  closeTrashView();
  $("settings").classList.add("hidden");
  collapseHoverRail();
  groupsOpen = true;
  $("groups").classList.remove("hidden");
  $("main").classList.add("groups-open");
  $("search-q").placeholder = "search the vault…";
  $("filters").classList.add("groups-mode");
  setViewRefreshSlot(true);
  if (lastMeta) renderGroupChips();
  loadGroupRandom();
  pushView("groups");
}
$("btn-groups").addEventListener("click", (e) => {
  e.stopPropagation();
  const view = $("groups");
  if (view.classList.contains("hidden")) openGroupsView();
});

/* One cluster at a time. No tile list, no cluster names. */
async function loadGroupRandom() {
  const status = $("groups-status");
  status.className = "import-status";
  status.textContent = "loading cluster…";
  const params = new URLSearchParams({ min_size: "6", limit: "120" });
  if (groupsSeen.size) params.set("exclude", Array.from(groupsSeen).slice(-12).join(","));
  for (const k of ["kind", "source", "hue", "tag"]) {
    if (groupsFilter[k]) params.set(k, groupsFilter[k]);
  }
  try {
    const data = await api("/api/groups/random?" + params.toString());
    if (data.error) throw new Error(data.error);
    if (data.cluster && data.cluster.id != null) {
      groupsSeen.add(String(data.cluster.id));
      while (groupsSeen.size > 24) groupsSeen.delete(groupsSeen.values().next().value);
    }
    renderGroup(data.cluster || {}, data.items || [], data.cluster ? data.cluster.matched : -1);
  } catch (err) {
    if (err.message === "auth required") return;
    status.className = "import-status error";
    status.textContent = "cluster failed: " + err.message;
  }
}

/* Filters narrow the cluster on screen. If nothing survives them, the server
   is asked for a cluster that does have matches. */
async function applyGroupsFilters() {
  if (!groupsCluster || !groupsCluster.id) return loadGroupRandom();
  const params = new URLSearchParams({ limit: "120" });
  for (const k of ["kind", "source", "hue", "tag"]) {
    if (groupsFilter[k]) params.set(k, groupsFilter[k]);
  }
  try {
    const data = await api("/api/groups/" + encodeURIComponent(groupsCluster.id) + "?" + params.toString());
    if (data.error) throw new Error(data.error);
    const items = data.items || [];
    const active = Object.values(groupsFilter).some(Boolean);
    if (!items.length && active) return loadGroupRandom();
    renderGroup(data.cluster || groupsCluster, items, data.cluster ? data.cluster.matched : -1);
  } catch (err) {
    if (err.message === "auth required") return;
    $("groups-status").textContent = "filter failed: " + err.message;
  }
}

function renderGroup(cluster, items, matched) {
  groupsCluster = cluster;
  groupsItems = Array.isArray(items) ? items : [];
  groupsMatched = typeof matched === "number" ? matched : -1;
  const total = Number(cluster.size || 0);
  const shown = groupsItems.length;
  $("groups-status").className = "import-status";
  $("groups-status").textContent = total > shown
    ? `${shown} of ${total}`
    : `${shown} ${shown === 1 ? "card" : "cards"}`;
  const wrap = $("groups-items");
  if (!shown) {
    wrap.innerHTML = '<div class="groups-empty">nothing in this cluster — next for another.</div>';
    wrap.style.height = "";
    return;
  }
  wrap.innerHTML = groupsItems.map(cardHtml).join("");
  wrap.querySelectorAll(".card-img.placeholder[data-media]").forEach((ph) => {
    if (!ph.dataset.loaded) observeCardLoad(ph);
  });
  requestAnimationFrame(groupsMasonry);
}

function groupChipRow(container, name, options, current) {
  if (!container) return;
  container.innerHTML = "";
  const all = document.createElement("button");
  all.className = "chip" + (current === "" ? " active" : "");
  all.textContent = "all";
  all.addEventListener("click", () => {
    groupsFilter[name] = "";
    renderGroupChips();
    applyGroupsFilters();
  });
  container.appendChild(all);
  for (const [value, count] of options) {
    const btn = document.createElement("button");
    btn.className = "chip" + (current === value ? " active" : "");
    btn.appendChild(document.createTextNode(value));
    if (typeof count === "number") {
      const cnt = document.createElement("span");
      cnt.className = "cnt";
      cnt.textContent = count;
      btn.appendChild(cnt);
    }
    btn.addEventListener("click", () => {
      groupsFilter[name] = current === value ? "" : value;
      renderGroupChips();
      applyGroupsFilters();
    });
    container.appendChild(btn);
  }
}

function renderGroupChips() {
  if (!lastMeta) return;
  groupChipRow($("chips-kind"), "kind", lastMeta.kinds || [], groupsFilter.kind);
  groupChipRow($("chips-source"), "source", lastMeta.sources || [], groupsFilter.source);
  groupChipRow($("chips-hue"), "hue", lastMeta.hues || [], groupsFilter.hue);
  groupChipRow($("chips-tag"), "tag", lastMeta.tags || [], groupsFilter.tag);
}

function groupsMasonry() {
  const grid = $("groups-items");
  if (!grid) return;
  const cards = [...grid.querySelectorAll(".card")];
  if (!cards.length) { grid.style.height = ""; return; }
  const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--grid-gap")) || 12;
  const padL = parseFloat(getComputedStyle(grid).paddingLeft) || 0;
  const padR = parseFloat(getComputedStyle(grid).paddingRight) || 0;
  const padT = parseFloat(getComputedStyle(grid).paddingTop) || 0;
  const width = grid.clientWidth - padL - padR;
  const target = width <= 600 ? Math.min(settings.gridSize, 150) : settings.gridSize;
  const maxCols = width <= 600 ? 2 : 6;
  const cols = Math.max(1, Math.min(maxCols, Math.floor((width + gap) / (target + gap))));
  const colW = (width - gap * (cols - 1)) / cols;
  const tops = new Array(cols).fill(0);
  for (const card of cards) {
    card.style.width = colW + "px";
    let best = 0;
    for (let i = 1; i < cols; i++) if (tops[i] < tops[best]) best = i;
    card.style.left = (padL + best * (colW + gap)) + "px";
    card.style.top = (padT + tops[best]) + "px";
    tops[best] += card.offsetHeight + gap;
  }
  grid.style.height = (padT + Math.max(...tops, 0)) + "px";
}

/* Delegated, bound once — re-renders must not stack listeners. */
$("groups-shuffle").addEventListener("click", (e) => {
  e.stopPropagation();
  loadGroupRandom();
});
$("groups-items").addEventListener("click", async (e) => {
  const hoverBtn = e.target.closest(".card-hover-btn");
  const card = e.target.closest(".card");
  if (hoverBtn) {
    e.preventDefault();
    e.stopPropagation();
    const id = hoverBtn.dataset.cardId;
    if (!id) return;
    if (hoverBtn.classList.contains("card-hover-open")) {
      openExternalUrl(hoverBtn.dataset.url);
      return;
    }
    if (hoverBtn.classList.contains("card-hover-copy")) {
      copyCardContent(id).then((message) => {
        flashHoverButton(hoverBtn, true, message);
      }).catch((err) => {
        flashHoverButton(hoverBtn, false, err.message);
      });
      return;
    }
    if (hoverBtn.classList.contains("card-hover-inspire")) {
      $("sort-menu").classList.add("hidden");
      $("filters").classList.add("collapsed");
      closeGroupsView();
      openInspireView();
      runInspire("", id);
      return;
    }
    if (hoverBtn.classList.contains("card-hover-delete")) {
      const ok = await deleteItem(id);
      if (ok) {
        const c = hoverBtn.closest(".card");
        if (c) { c.remove(); groupsMasonry(); }
      }
      return;
    }
    return;
  }
  if (!card) return;
  if (e.button !== 0) return;
  if (card.classList.contains("card-unavailable")) return;
  pulseCard(card);
  openDetail(card.dataset.id);
});

/* ---------- avatar menu / logout ---------- */
$("avatar").addEventListener("click", (e) => {
  e.stopPropagation();
  $("sort-menu").classList.add("hidden");
  closeDetail();
  closeCleanupView();
  closeInspireView();
  closeFollowupsView(); // settings is a full-page view too
  closeTrashView();
  closeGroupsView();
  $("filters").classList.add("collapsed");
  $("settings").classList.remove("hidden");
  syncSettingsUI();
  pushView("settings");
});

// Toolbar clicks (search/filters/sort/grid-size, including their icon SVGs)
// always mark a recent hit. The outside-click handler below honors it, so
// using any vault control never closes the follow-ups or cleanup views.
document.querySelectorAll("#toolbar, #filters, #sort-menu").forEach((el) => {
  el.addEventListener("click", () => { window.__toolbarHit = Date.now(); }, { passive: true });
});

document.addEventListener("click", (e) => {
  const toolbarHit = window.__toolbarHit && Date.now() - window.__toolbarHit < 700;
  // A click that re-renders its own list detaches the clicked node mid-dispatch;
  // closest() then fails on the orphaned target and the view would wrongly close.
  const gone = e.target instanceof Element && !e.target.isConnected;
  if (!e.target.closest("#sort-menu") && !e.target.closest("#btn-sort")) $("sort-menu").classList.add("hidden");
  // Cleanup dropdown panels: close when clicking outside the panel + its toggle
  // (the shared toolbar buttons on desktop, the cleanup row buttons on mobile).
  if (!e.target.closest("#cleanup-filter-menu") && !e.target.closest("#btn-filters") && !e.target.closest("#btn-cleanup-filters")) closeCleanupFilterMenu();
  if (!e.target.closest("#cleanup-sort-menu") && !e.target.closest("#btn-sort") && !e.target.closest("#btn-cleanup-sort")) closeCleanupSortMenu();
  if (!gone && !e.target.closest("#cleanup") && !e.target.closest("#btn-cleanup") && !e.target.closest("#followups") && !e.target.closest("#btn-followups") && !e.target.closest("#detail") && !e.target.closest(".topbar") && !toolbarHit) {
    closeCleanupView();
  }
  if (!gone && !e.target.closest("#followups") && !e.target.closest("#btn-followups") && !e.target.closest("#detail") && !e.target.closest("#recipe-reader") && !e.target.closest(".topbar") && !toolbarHit) {
    closeFollowupsView();
  }
  if (!gone && !e.target.closest("#groups") && !e.target.closest("#btn-groups") && !e.target.closest("#detail") && !e.target.closest(".topbar") && !toolbarHit) {
    closeGroupsView();
  }
  if (!gone && !e.target.closest("#trash") && !e.target.closest("#btn-trash") && !e.target.closest("#detail") && !e.target.closest(".topbar") && !toolbarHit) {
    closeTrashView();
  }
  if (!gone && !e.target.closest("#inspire") && !e.target.closest("#btn-inspire") && !e.target.closest("#search-form") && !e.target.closest("#detail") && !e.target.closest(".topbar")) closeInspireView();
});
function logout() {
  localStorage.removeItem(AUTH_KEY);
  state.creds = null;
  clearBlobCache();
  $("settings").classList.add("hidden");
  showLogin();
}

/* ---------- settings store ---------- */
const SETTINGS_KEY = "te_settings_v1";
const GRID_SIZE = { min: 120, max: 420, step: 10, default: 260 };
const GRID_SIZE_PRESETS = { dense: 150, regular: 260, exploded: 380 };
function gridSizePreset(value) {
  const n = Number(value);
  const entries = Object.entries(GRID_SIZE_PRESETS);
  return entries.reduce((best, entry) => Math.abs(entry[1] - n) < Math.abs(best[1] - n) ? entry : best, entries[1])[0];
}
function normalizeGridSize(value) {
  const legacy = { compact: 220, standard: 300, spacious: 360 };
  const raw = typeof value === "string" && GRID_SIZE_PRESETS[value]
    ? GRID_SIZE_PRESETS[value]
    : typeof value === "string" && legacy[value]
      ? legacy[value]
      : Number(value);
  const n = Number.isFinite(raw) ? raw : GRID_SIZE.default;
  return GRID_SIZE_PRESETS[gridSizePreset(n)] || GRID_SIZE.default;
}
const DEFAULT_SETTINGS = {
  theme: "dark",
  style: "dithering",
  animations: true,
  gridSize: GRID_SIZE.default,
  sort: "random",
  sidebarOpen: false,
  rows: { meta: false, note: false, explainer: false, similar: true },
};
function loadSettings() {
  try {
    const s = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}");
    return {
      ...DEFAULT_SETTINGS,
      ...s,
      style: DEFAULT_SETTINGS.style,  // only dithering; flat was removed
      sort: ["random", "alpha_asc", "alpha_desc", "recent", "oldest", "frequent"].includes(s.sort) ? s.sort : DEFAULT_SETTINGS.sort,
      gridSize: normalizeGridSize(s.gridSize ?? s.density),
      rows: { ...DEFAULT_SETTINGS.rows, ...(s.rows || {}) },
    };
  } catch (e) {
    return { ...DEFAULT_SETTINGS, rows: { ...DEFAULT_SETTINGS.rows } };
  }
}
let settings = loadSettings();
state.sort = settings.sort;
let settingsSyncTimer = null;
let settingsSyncReady = false;
function saveSettings() {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  if (!settingsSyncReady || !state.creds) return;
  clearTimeout(settingsSyncTimer);
  settingsSyncTimer = setTimeout(() => {
    api("/api/settings", { method: "PUT", body: { settings } }).catch(() => {});
  }, 180);
}
async function loadServerSettings() {
  try {
    const data = await api("/api/settings");
    if (data.settings && typeof data.settings === "object") {
      const remote = data.settings;
  settings = {
        ...DEFAULT_SETTINGS,
        ...remote,
        theme: remote.theme === "light" ? "light" : "dark",
        style: DEFAULT_SETTINGS.style,  // only dithering; flat was removed
        animations: remote.animations !== false,
        sort: ["random", "alpha_asc", "alpha_desc", "recent", "oldest", "frequent"].includes(remote.sort) ? remote.sort : DEFAULT_SETTINGS.sort,
        gridSize: normalizeGridSize(remote.gridSize ?? remote.density),
        rows: { ...DEFAULT_SETTINGS.rows, ...(remote.rows || {}) },
      };
      state.sort = settings.sort;
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
      applySettings();
      if (typeof syncGridSizeButton === "function") syncGridSizeButton();
    } else {
      settingsSyncReady = true;
      saveSettings();
      return;
    }
    settingsSyncReady = true;
  } catch (err) {
    settingsSyncReady = true;
  }
}
function applySettings() {
  document.documentElement.dataset.theme = settings.theme;
  document.documentElement.dataset.style = settings.style;
  document.documentElement.classList.toggle("no-anim", !settings.animations);
}
applySettings();
// NOTE: syncGridSizeButton() and initHuePopup() read ICONS/HUE_COLORS,
// which are const-declared LATER in this file. Calling them here hits the
// temporal dead zone and kills the whole script (blank app after refresh).
// They run at the top of boot() instead.

/* ---------- settings ui ---------- */
const $s = (sel) => $("settings").querySelector(sel);
function syncSettingsUI() {
  const username = $("settings-username");
  if (username) username.textContent = state.creds ? state.creds.user : "";
  for (const [seg, val] of [["#seg-theme", settings.theme]]) {
    $s(seg).querySelectorAll(".seg-btn").forEach((b) => b.classList.toggle("active", b.dataset.v === val));
  }
  syncGridSizeButton();
  const sws = { "#sw-anim": settings.animations, "#sw-sidebar": settings.sidebarOpen };
  for (const k of Object.keys(DEFAULT_SETTINGS.rows)) sws["#sw-row-" + k] = settings.rows[k];
  for (const [sel, val] of Object.entries(sws)) {
    const el = $s(sel);
    if (el) el.setAttribute("aria-checked", String(val));
  }
}
$("btn-settings-logout").addEventListener("click", logout);
// NOTE: no click-outside-to-close on settings anymore. It is a full-page
// view like follow-ups/cleanup: only the X in its toolbar closes it.
$s("#seg-theme").querySelectorAll(".seg-btn").forEach((b) => b.addEventListener("click", () => {
  settings.theme = b.dataset.v; saveSettings(); applySettings(); syncSettingsUI();
}));
/* topbar grid-size toggle: dense -> regular -> exploded -> dense */
const GRID_ORDER = ["dense", "regular", "exploded"];
function syncGridSizeButton() {
  const btn = $("btn-gridsize");
  if (!btn) return;
  const preset = gridSizePreset(settings.gridSize);
  // swap the visible icon to match the current density
  const icWrap = btn.querySelector(".ic");
  const variant = ICONS["grid-" + preset] || ICONS["grid-regular"];
  if (icWrap) icWrap.innerHTML = variant;
  btn.title = "grid size: " + preset;
  btn.setAttribute("aria-label", "grid size: " + preset);
}
$("btn-gridsize").addEventListener("click", (e) => {
  e.stopPropagation();
  $("sort-menu").classList.add("hidden");
  if (isFollowupsOpen()) {
    // In the follow-ups the button drives the same density setting as the grid.
    const preset = gridSizePreset(settings.gridSize);
    const next = GRID_ORDER[(GRID_ORDER.indexOf(preset) + 1) % GRID_ORDER.length];
    settings.gridSize = GRID_SIZE_PRESETS[next] || GRID_SIZE.default;
    saveSettings();
    syncGridSizeButton();
    fuMasonry();
    return;
  }
  if ($("main").classList.contains("cleanup-open")) {
    const preset = gridSizePreset(settings.gridSize);
    const next = GRID_ORDER[(GRID_ORDER.indexOf(preset) + 1) % GRID_ORDER.length];
    settings.gridSize = GRID_SIZE_PRESETS[next] || GRID_SIZE.default;
    saveSettings();
    syncGridSizeButton();
    applyCleanupGridSize();
    cleanupMasonry();
    return;
  }
  if (isTrashOpen()) {
    const preset = gridSizePreset(settings.gridSize);
    const next = GRID_ORDER[(GRID_ORDER.indexOf(preset) + 1) % GRID_ORDER.length];
    settings.gridSize = GRID_SIZE_PRESETS[next] || GRID_SIZE.default;
    saveSettings();
    syncGridSizeButton();
    trashMasonry();
    return;
  }
  const preset = gridSizePreset(settings.gridSize);
  const next = GRID_ORDER[(GRID_ORDER.indexOf(preset) + 1) % GRID_ORDER.length];
  settings.gridSize = GRID_SIZE_PRESETS[next] || GRID_SIZE.default;
  saveSettings();
  syncGridSizeButton();
  masonry();
});
dbgMark("gridsize-handler-bound");
function bindSwitch(sel, key) {
  $s(sel).addEventListener("click", () => {
    settings[key] = !settings[key]; saveSettings(); applySettings(); syncSettingsUI();
  });
}
bindSwitch("#sw-anim", "animations");
bindSwitch("#sw-sidebar", "sidebarOpen");
for (const k of Object.keys(DEFAULT_SETTINGS.rows)) {
  $s("#sw-row-" + k).addEventListener("click", () => {
    settings.rows[k] = !settings.rows[k]; saveSettings(); syncSettingsUI();
  });
}
$("btn-settings-reset").addEventListener("click", () => {
  settings = { ...DEFAULT_SETTINGS, rows: { ...DEFAULT_SETTINGS.rows } };
  state.sort = settings.sort;
  saveSettings(); applySettings(); syncSettingsUI(); syncSortUI(); masonry();
});

/* ---------- sort ---------- */
const SORT_LABELS = {
  random: "random",
  alpha_asc: "alphabetical A to Z",
  alpha_desc: "alphabetical Z to A",
  recent: "recent to last",
  oldest: "last to recent",
  frequent: "frequently clicked",
};
function syncSortUI() {
  document.querySelectorAll("#sort-menu .sort-option").forEach((option) => {
    option.classList.toggle("active", option.dataset.sort === state.sort);
  });
  const button = $("btn-sort");
  if (button) button.title = "sort: " + (SORT_LABELS[state.sort] || SORT_LABELS.random);
}
$("btn-sort").addEventListener("click", (e) => {
  e.stopPropagation();
  // In the cleanup view the main sort button drives the cleanup sort menu.
  if ($("main").classList.contains("cleanup-open")) {
    const menu = $("cleanup-sort-menu");
    closeCleanupFilterMenu();
    renderCleanupSortMenu();
    menu.classList.toggle("hidden");
    syncCleanupSortUI();
    syncViewPanelOffset();
    return;
  }
  const menu = $("sort-menu");
  if (!isTrashOpen()) closeCleanupView();
  closeInspireView();
  $("settings").classList.add("hidden");
  $("filters").classList.add("collapsed");
  menu.classList.toggle("hidden");
  syncSortUI();
  syncViewPanelOffset();
});
document.querySelectorAll(".sort-option").forEach((option) => option.addEventListener("click", (e) => {
  e.stopPropagation();
  state.sort = option.dataset.sort || "random";
  settings.sort = state.sort;
  saveSettings();
  // Keep the sort menu open after selecting, exactly like the filter drawer:
  // the user can try several options and closes it with the button/outside.
  state.offset = 0;
  syncSortUI();
  if (isFollowupsOpen()) renderFollowups();
  else if (isTrashOpen()) renderTrash();
  else refresh({ replace: true });
}));

/* ---------- icons ---------- */
const ICONS = {
  filters: '<svg viewBox="0 0 24 24"><polygon points="22 3 2 3 10 12.46 10 19 14 21 14 12.46 22 3"/></svg>',
  x: '<svg viewBox="0 0 24 24"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>',
  zap: '<svg viewBox="0 0 24 24"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
  download: '<svg viewBox="0 0 24 24"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="3" x2="12" y2="15"/></svg>',
  refresh: '<svg viewBox="0 0 24 24"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>',
  trash: '<svg viewBox="0 0 24 24"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>',
  copy: '<svg viewBox="0 0 24 24"><rect x="9" y="9" width="11" height="11" rx="1"/><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"/></svg>',
  external: '<svg viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
  restore: '<svg viewBox="0 0 24 24"><path d="M4 16h16v3H4"/><path d="M12 15V7"/><path d="M12 7l-4 3 8 0"/></svg>',
  "chev-r": '<svg viewBox="0 0 24 24"><polyline points="9 6 15 12 9 18"/></svg>',
  "chev-l": '<svg viewBox="0 0 24 24"><polyline points="15 6 9 12 15 18"/></svg>',
  "panel-open": '<svg viewBox="0 0 24 24"><polyline points="11 17 6 12 11 7"/><polyline points="18 17 13 12 18 7"/></svg>',
  "panel-close": '<svg viewBox="0 0 24 24"><polyline points="13 17 18 12 13 7"/><polyline points="6 17 11 12 6 7"/></svg>',
  instagram: '<svg viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18"/><circle cx="12" cy="12" r="4"/><circle cx="17.5" cy="6.5" r="1" fill="currentColor" stroke="none"/></svg>',
  plus: '<svg viewBox="0 0 24 24"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>',
  "grid-dense": '<svg viewBox="0 0 24 24"><path fill="currentColor" stroke="none" d="M4 4h4v4H4zm6 0h4v4h-4zm6 0h4v4h-4zM4 10h4v4H4zm6 0h4v4h-4zm6 0h4v4h-4zM4 16h4v4H4zm6 0h4v4h-4zm6 0h4v4h-4z"/></svg>',
  "grid-regular": '<svg viewBox="0 0 24 24"><path fill="currentColor" stroke="none" d="M3 11V3h8v8zm0 10v-8h8v8zm10-10V3h8v8zm0 10v-8h8v8z"/></svg>',
  "grid-exploded": '<svg viewBox="0 0 24 24"><path fill="currentColor" stroke="none" d="M2 11V4h20v7zm8 9v-7h12v7zm-8 0v-7h6v7z"/></svg>',
};
function ic(name, cls = "") {
  return `<span class="ic ${cls}">${ICONS[name] || ""}</span>`;
}

async function copyText(text) {
  const value = String(text || "");
  if (!value) throw new Error("nothing to copy");
  if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch (err) {
      // Plain HTTP and browser permission rules can reject the Clipboard API.
    }
  }
  const area = document.createElement("textarea");
  area.value = value;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.left = "-9999px";
  area.style.top = "0";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.focus();
  area.select();
  area.setSelectionRange(0, area.value.length);
  let copied = false;
  try {
    copied = document.execCommand("copy");
  } finally {
    area.remove();
  }
  if (!copied) throw new Error("copy is not available in this browser");
}

async function copyCardContent(id) {
  const item = await api("/api/items/" + encodeURIComponent(id));
  const media = item.cover && (item.cover.file || item.cover.thumb);
  if (media && navigator.clipboard && window.ClipboardItem) {
    try {
      const res = await fetch(mediaUrl(media), { headers: authHeader() });
      if (!res.ok) throw new Error("media " + res.status);
      const blob = await res.blob();
      if (!blob.type.startsWith("image/")) throw new Error("clipboard image is not supported");
      let imageBlob = blob;
      if (blob.type !== "image/png") {
        const image = await createImageBitmap(blob);
        const canvas = document.createElement("canvas");
        canvas.width = image.width; canvas.height = image.height;
        canvas.getContext("2d").drawImage(image, 0, 0);
        imageBlob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
        image.close();
      }
      await navigator.clipboard.write([new ClipboardItem({ "image/png": imageBlob })]);
      return "picture copied";
    } catch (err) {
      // fall through to the text/link clipboard below
    }
  }
  // No image clipboard (Firefox/Zen, plain-HTTP context) or the image fetch
  // failed: still copy something useful. Prefer the source link, then text.
  const url = (item.url || "").trim();
  const text = [item.title, item.text].filter((value) => String(value || "").trim()).join("\n\n").trim();
  const payload = url || text;
  if (!payload) throw new Error("nothing to copy");
  await copyText(payload);  // textarea fallback works everywhere
  return url ? "image link copied" : "text copied";
}

/* ---------- filters toggle ---------- */
$("btn-filters").addEventListener("click", (e) => {
  e.stopPropagation();
  // In the cleanup view the main filter button drives the cleanup evidence
  // panel (same toolbar, same slot — no duplicate buttons).
  if ($("main").classList.contains("cleanup-open")) {
    const menu = $("cleanup-filter-menu");
    if (menu.classList.contains("hidden")) openCleanupFilterMenu();
    else closeCleanupFilterMenu();
    return;
  }
  const f = $("filters");
  const open = f.classList.toggle("collapsed") === false;
  $("filters-ic").innerHTML = open ? ICONS.x : ICONS.filters;
  $("btn-filters").title = open
    ? (isTrashOpen() ? "close trash filters" : isFollowupsOpen() ? "close follow-up filters" : "close filters")
    : (isTrashOpen() ? "filter trash" : isFollowupsOpen() ? "filter follow-ups" : "filters");
  syncViewPanelOffset();
});

/* ---------- meta / filter chips ---------- */
const HUE_DOTS = {
  "warm-neutral": "#c9a37e", orange: "#d98b4a", amber: "#d9b45a", yellow: "#d9c96a",
  olive: "#a8b06a", green: "#7fae7f", teal: "#6fb3a8", cyan: "#7fb5c9",
  blue: "#7f9fd9", purple: "#9b7fd9", pink: "#d98fae", magenta: "#d97fb0",
  red: "#d97a7a", "neutral-dark": "#5a5363", "neutral-light": "#c7c0d1",
  "cool-neutral": "#9aa3b8",
};

function hydrateMeta(meta) {
  lastMeta = meta;
  const groups = [
    ["kind", meta.kinds],
    ["hue", meta.hues],
    ["source", meta.sources],
    ["tag", meta.tags],
  ];
  for (const [group, entries] of groups) {
    if (group !== "hue") renderChips(group, entries);
  }
  refreshHuePopup(meta.hues);
  $("meta-count").textContent = Number(meta.total || 0).toLocaleString();
}

function renderChips(group, entries) {
  const wrap = $("chips-" + group);
  const cur = state.filter[group];
  if (!Array.isArray(entries)) entries = []; // never nuke the row on bad data
  const all = document.createElement("button");
  all.className = "chip" + (cur === "" ? " active" : "");
  all.textContent = "all";
  all.addEventListener("click", () => {
    state.filter[group] = "";
    state.offset = 0;
    refresh({ replace: true });
    if (lastMeta) renderChips(group, lastMeta[group + "s"]);
  });
  wrap.innerHTML = "";
  wrap.appendChild(all);
  for (const [value, count] of entries.slice(0, group === "tag" ? 14 : 10)) {
    if (group === "source" && value === "mymind") continue; // legacy rows
    const btn = document.createElement("button");
    btn.className = "chip" + (cur === value ? " active" : "");
    if (group === "hue" && HUE_DOTS[value]) {
      const dot = document.createElement("span");
      dot.className = "dot";
      dot.style.background = HUE_DOTS[value];
      btn.appendChild(dot);
    }
    const label = document.createElement("span");
    label.textContent = group === "source"
      ? value.charAt(0).toUpperCase() + value.slice(1)
      : value;
    btn.appendChild(label);
    const cnt = document.createElement("span");
    cnt.className = "cnt";
    cnt.textContent = count;
    btn.appendChild(cnt);
    btn.addEventListener("click", () => {
      state.filter[group] = state.filter[group] === value ? "" : value;
      state.offset = 0;
      refresh({ replace: true });
      if (lastMeta) renderChips(group, lastMeta[group + "s"]);
    });
    wrap.appendChild(btn);
  }
}

/* ---------- hue popup picker ---------- */
const HUE_COLORS = {
  red: "#d94f4f", orange: "#d98b3a", amber: "#d9a83a", yellow: "#d9c94a",
  olive: "#9aa83a", green: "#5fae5f", teal: "#5fb3a0", cyan: "#5fb5d9",
  blue: "#5f8fd9", purple: "#8b5fd9", magenta: "#d95fb0", pink: "#d96fa0",
  "warm-neutral": "#c9a37e", "neutral-dark": "#4a4353",
  "neutral-light": "#c7c0d1", "cool-neutral": "#8a93a8",
};

function initHuePopup() {
  const trigger = $("hue-trigger");
  const popup = $("hue-popup");
  const grid = $("hue-popup-grid");
  const clearBtn = $("hue-popup-clear");
  if (!trigger || !popup || !grid) return;

  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    popup.classList.toggle("hidden");
    if (!popup.classList.contains("hidden")) repaintHuePicker();
  });

  document.addEventListener("click", (e) => {
    if (!popup.classList.contains("hidden") && !popup.contains(e.target) && e.target !== trigger) {
      popup.classList.add("hidden");
    }
  });

  clearBtn.addEventListener("click", () => {
    state.filter.hue = "";
    state.offset = 0;
    refresh({ replace: true });
    updateHueTrigger();
    popup.classList.add("hidden");
  });

  initHuePicker();
}

/* ---------- free hue picker (SV pad + spectrum + hex) ---------- */
let huePickerHsv = { h: 0, s: 1, v: 1 };
let hueSnapTimer = null;

function hsvToRgb(h, s, v) {
  const f = (n) => {
    const k = (n + h * 6) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return [Math.round(f(5) * 255), Math.round(f(3) * 255), Math.round(f(1) * 255)];
}
function rgbToHsv(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn;
  let h = 0;
  if (d) {
    if (mx === r) h = ((g - b) / d) % 6;
    else if (mx === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  return { h, s: mx ? d / mx : 0, v: mx };
}
function hsvToHex(h, s, v) {
  const [r, g, b] = hsvToRgb(h, s, v);
  return "#" + [r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("");
}
function hexToHsv(hex) {
  let raw = (hex || "").trim().replace(/^#/, "");
  if (raw.length === 3) raw = raw.split("").map((c) => c + c).join("");
  if (!/^[0-9a-fA-F]{6}$/.test(raw)) return null;
  const r = parseInt(raw.slice(0, 2), 16);
  const g = parseInt(raw.slice(2, 4), 16);
  const b = parseInt(raw.slice(4, 6), 16);
  return rgbToHsv(r, g, b);
}

function repaintHuePicker() {
  const pad = $("hue-pad"), spec = $("hue-spectrum"), hexIn = $("hue-hex");
  const padMarker = $("hue-pad-marker"), specMarker = $("hue-spec-marker");
  if (!pad || !spec || !hexIn) return;
  const { h, s, v } = huePickerHsv;
  const hueCss = `hsl(${Math.round(h)}, 100%, 50%)`;
  pad.style.background =
    `linear-gradient(to top, #000, rgba(0,0,0,0)), linear-gradient(to right, #fff, rgba(255,255,255,0)), ${hueCss}`;
  spec.style.background = "linear-gradient(to right, #f00 0%, #ff0 17%, #0f0 33%, #0ff 50%, #00f 67%, #f0f 83%, #f00 100%)";
  if (padMarker) {
    padMarker.style.left = `${s * 100}%`;
    padMarker.style.top = `${(1 - v) * 100}%`;
  }
  if (specMarker) specMarker.style.left = `${(h / 360) * 100}%`;
  if (hexIn.value !== hsvToHex(h, s, v) && document.activeElement !== hexIn) {
    hexIn.value = hsvToHex(h, s, v);
  }
  scheduleHueSnap();
}

function scheduleHueSnap() {
  clearTimeout(hueSnapTimer);
  hueSnapTimer = setTimeout(async () => {
    const snap = $("hue-snap");
    if (!snap) return;
    const hex = hsvToHex(huePickerHsv.h, huePickerHsv.s, huePickerHsv.v).slice(1);
    try {
      const res = await api("/api/hue-from-hex?hex=" + hex);
      if (res && res.hue) {
        const c = HUE_COLORS[res.hue] || "#888";
        snap.innerHTML = `<span class="dot" style="background:${c}"></span>filter → ${res.hue}`;
      }
    } catch (e) {
      snap.textContent = "";
    }
  }, 160);
}

async function applyHuePicker() {
  const hex = hsvToHex(huePickerHsv.h, huePickerHsv.s, huePickerHsv.v).slice(1);
  try {
    const res = await api("/api/hue-from-hex?hex=" + hex);
    if (res && res.hue) {
      state.filter.hue = res.hue;
      state.offset = 0;
      await refresh({ replace: true });
      updateHueTrigger();
    }
  } catch (e) {
    // leave filter unchanged
  }
  $("hue-popup").classList.add("hidden");
}

function initHuePicker() {
  const pad = $("hue-pad"), spec = $("hue-spectrum"), hexIn = $("hue-hex");
  const applyBtn = $("hue-apply");
  if (!pad || !spec || !hexIn) return;

  let dragging = null;

  const padPos = (ev) => {
    const r = pad.getBoundingClientRect();
    const s = Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width));
    const v = 1 - Math.min(1, Math.max(0, (ev.clientY - r.top) / r.height));
    return { s, v };
  };
  pad.addEventListener("pointerdown", (ev) => {
    dragging = "pad";
    pad.setPointerCapture(ev.pointerId);
    const { s, v } = padPos(ev);
    huePickerHsv.s = s; huePickerHsv.v = v;
    repaintHuePicker();
  });
  pad.addEventListener("pointermove", (ev) => {
    if (dragging !== "pad") return;
    const { s, v } = padPos(ev);
    huePickerHsv.s = s; huePickerHsv.v = v;
    repaintHuePicker();
  });
  pad.addEventListener("pointerup", () => {
    if (dragging === "pad") { dragging = null; applyHuePicker(); }
  });

  const specPos = (ev) => {
    const r = spec.getBoundingClientRect();
    return Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width));
  };
  spec.addEventListener("pointerdown", (ev) => {
    dragging = "spec";
    spec.setPointerCapture(ev.pointerId);
    huePickerHsv.h = specPos(ev) * 360;
    repaintHuePicker();
  });
  spec.addEventListener("pointermove", (ev) => {
    if (dragging !== "spec") return;
    huePickerHsv.h = specPos(ev) * 360;
    repaintHuePicker();
  });
  spec.addEventListener("pointerup", () => {
    if (dragging === "spec") { dragging = null; applyHuePicker(); }
  });

  hexIn.addEventListener("input", () => {
    const hsv = hexToHsv(hexIn.value);
    if (hsv) {
      huePickerHsv = hsv;
      repaintHuePicker();
    }
  });
  hexIn.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") {
      const hsv = hexToHsv(hexIn.value);
      if (hsv) { huePickerHsv = hsv; repaintHuePicker(); }
      applyHuePicker();
    }
  });
  if (applyBtn) applyBtn.addEventListener("click", applyHuePicker);
  repaintHuePicker();
}

function buildHuePopupGrid(entries) {
  const grid = $("hue-popup-grid");
  if (!grid) return;
  grid.innerHTML = "";
  if (!Array.isArray(entries)) entries = [];
  const cur = state.filter.hue;

  for (const [value, count] of entries) {
    const swatch = document.createElement("button");
    swatch.className = "hue-swatch" + (cur === value ? " active" : "");
    swatch.title = `${value} · ${count}`;
    swatch.style.background = HUE_COLORS[value] || "#888";
    swatch.addEventListener("click", () => {
      state.filter.hue = state.filter.hue === value ? "" : value;
      state.offset = 0;
      refresh({ replace: true });
      updateHueTrigger();
      $("hue-popup").classList.add("hidden");
    });
    grid.appendChild(swatch);
  }
}

function updateHueTrigger() {
  const trigger = $("hue-trigger");
  if (!trigger) return;
  const cur = state.filter.hue;
  if (cur && HUE_COLORS[cur]) {
    trigger.innerHTML = `<span class="dot" style="background:${HUE_COLORS[cur]}"></span>${cur}`;
    trigger.classList.add("active");
  } else {
    trigger.textContent = "all";
    trigger.classList.remove("active");
  }
}

// Expose for hydrateMeta to rebuild the popup grid when meta refreshes
function refreshHuePopup(entries) {
  buildHuePopupGrid(entries);
  updateHueTrigger();
}

function wedgeClip(startDeg, endDeg) {
  const r = 50, cx = 50, cy = 50;
  const points = [`${cx} ${cy}`];
  const steps = 8;
  for (let i = 0; i <= steps; i++) {
    const a = (startDeg + (endDeg - startDeg) * i / steps) * Math.PI / 180 - Math.PI / 2;
    points.push(`${(cx + r * Math.cos(a)).toFixed(1)} ${(cy + r * Math.sin(a)).toFixed(1)}`);
  }
  return `polygon(${points.join(",")})`;
}

$("f-clear").addEventListener("click", () => {
  state.filter = { kind: "", hue: "", source: "", tag: "" };
  $("f-clear").classList.add("hidden");
  state.offset = 0;
  refresh({ replace: true });
  if (lastMeta) {
    for (const [group, entries] of [["kind", lastMeta.kinds], ["hue", lastMeta.hues], ["source", lastMeta.sources], ["tag", lastMeta.tags]]) {
      renderChips(group, entries);
    }
  }
});

/* ---------- search / load ---------- */
$("search-form").addEventListener("submit", (e) => {
  e.preventDefault();
  doSearch();
});
$("search-q").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    doSearch();
  }
});
function doSearch() {
  const q = $("search-q").value.trim();
  if (isFollowupsOpen()) {
    fuQuery = q;
    renderFollowups();
    return;
  }
  // In the cleanup view the search box filters the loaded duplicate groups
  // client-side (same toolbar slot, no separate search box). Do NOT write
  // state.query — the vault search must stay untouched by a cleanup pass.
  if ($("main").classList.contains("cleanup-open")) {
    cleanupQuery = q;
    if (cleanupGroups.length) renderCleanupGroups(cleanupGroups);
    return;
  }
  // Groups view: clusters carry no names to filter, so the search box stays
  // inert here rather than writing vault search state.
  if (isGroupsOpen()) return;
  // Trash: client-side filter over the loaded trash items.
  if (isTrashOpen()) {
    trashQuery = q;
    renderTrash();
    return;
  }
  state.query = q;
  state.offset = 0;
  closeCleanupView();
  closeInspireView();
  refresh({ replace: true });
}

function showPageLoading() {
  const grid = $("grid");
  clearCardObserver();
  $("empty").classList.add("hidden");
  $("loadmore-wrap").classList.add("hidden");
  grid.classList.add("is-loading");
  grid.setAttribute("aria-busy", "true");
  grid.innerHTML = Array.from({ length: 12 }, (_, i) => `
    <div class="card card-skeleton" aria-hidden="true" style="--skeleton-delay:${i * 35}ms">
      <div class="skeleton-media"></div>
      <div class="skeleton-body">
        <span class="skeleton-line skeleton-line-wide"></span>
        <span class="skeleton-line skeleton-line-short"></span>
      </div>
    </div>`).join("");
  masonry();
}

function showSearchLoading() {
  showPageLoading();
}

function hideSearchLoading() {
  const grid = $("grid");
  grid.classList.remove("is-loading");
  grid.removeAttribute("aria-busy");
}

async function refresh({ replace = false } = {}) {
  if (state.loading) return;
  state.loading = true;
  const showLoading = replace && state.offset === 0;
  if (showLoading) showPageLoading();
  const f = state.filter;
  const params = new URLSearchParams({ offset: state.offset });
  if (state.query) {
    params.set("q", state.query);
    params.set("mode", "auto");
  }
  if (f.kind) params.set("kind", f.kind);
  if (f.hue) params.set("hue", f.hue);
  if (f.source) params.set("source", f.source);
  if (f.tag) params.set("tag", f.tag);
  params.set("sort", state.sort);
  if (!state.query) {
    params.set("seed", state.seed);
  }
  try {
    const ep = state.query ? "/api/search?" + params : "/api/items?" + params;
    const data = await api(ep);
    if (state.offset === 0) {
      $("grid").innerHTML = "";
      installQuickNote();
      hideSearchLoading();
      $("loadmore-wrap").classList.add("hidden");
      clearCardObserver();
      $("empty").classList.add("hidden");
    }
    // Only dedupe on append: offset 0 just cleared the grid, so the DOM can
    // hold fresh cards (e.g. a just-saved note) that a later feed page may
    // return again. Drop those ids here so scroll never duplicates.
    const rendered = new Set([...$("grid").querySelectorAll(".card[data-id]")].map((el) => el.dataset.id));
    const items = data.items.filter((it) => !rendered.has(it.id));
    $("grid").insertAdjacentHTML("beforeend", items.map(cardHtml).join(""));
    masonry(); // sync layout before paint — prevents top-left pile-up flash
    await hydrateCards(items);
    state.offset += data.items.length;
    // The search API and the library feed both report has_more now.
    state.hasMore = data.has_more !== undefined ? data.has_more : (data.items.length > 0);
    if (state.hasMore) {
      $("loadmore-wrap").classList.remove("hidden");
      setupScrollObserver();
    } else {
      $("loadmore-wrap").classList.add("hidden");
      teardownScrollObserver();
    }
    if (!data.items.length && state.offset === data.items.length) {
      $("empty").textContent = state.query || Object.values(f).some(Boolean)
        ? "nothing matches — adjust filters or the query."
        : "nothing here yet.";
      $("empty").classList.remove("hidden");
    }
    // Freshly imported items hit the grid before their background pipeline
    // job has downloaded media. A coverless card falls back to raw post text,
    // so poll those items and re-render each card in place when its cover
    // arrives (2026-09-01: import showed caption-text cards instead of shots).
    startCoverPoll();
  } catch (err) {
    hideSearchLoading();
    if (err.message === "auth required") return;
    $("empty").textContent = "error: " + err.message;
    $("empty").classList.remove("hidden");
  } finally {
    state.loading = false;
  }
}

/* ---------- infinite scroll ---------- */
let scrollObserver = null;

function setupScrollObserver() {
  if (scrollObserver) return; // already observing
  const sentinel = $("scroll-sentinel");
  if (!sentinel) return;
  scrollObserver = new IntersectionObserver((entries) => {
    if (entries[0].isIntersecting && !state.loading && state.hasMore) {
      refresh();
    }
  }, { rootMargin: "400px" });
  scrollObserver.observe(sentinel);
}

function teardownScrollObserver() {
  if (scrollObserver) {
    scrollObserver.disconnect();
    scrollObserver = null;
  }
}

const BRAND_MARK = '<img class="ui-mark" src="/assets/taste-engine-logo-transparent.png" alt="">';
const KIND_GLYPH = { image: BRAND_MARK, video: "▶", text: "¶", link: "↗" };

function hueFromId(id) {
  let h = 0;
  for (let i = 0; i < id.length; i += 2) h = (h * 31 + id.charCodeAt(i)) % 360;
  return h;
}

function coverColor(it) {
  const hue = it.cover && it.cover.hue;
  if (hue && HUE_DOTS[hue]) return HUE_DOTS[hue];
  return "hsl(" + hueFromId(it.id) + " 45% 62%)";
}

function coverAR(it) {
  const c = it.cover;
  if (c && c.width > 1 && c.height > 1) {
    const w = Math.min(4, Math.max(0.25, c.width / c.height));
    return (Math.round(w * 100) / 100) + "/1";
  }
  return "4/3";
}

function previewState(it) {
  if (it && it.cover && it.cover.thumb) return "ready";
  const job = it && it.job ? it.job.state : "";
  return job === "queued" || job === "working" ? "queued" : "unavailable";
}

function previewStatusHtml(it, compact = false) {
  const queued = previewState(it) === "queued";
  const label = queued ? "preview queued" : "media unavailable";
  const detail = queued ? "processing" : platformLabel(it && it.url);
  return `<div class="media-status${compact ? " compact" : ""}">
    <span class="media-status-mark${queued ? " is-loading" : ""}">${queued ? BRAND_MARK : "×"}</span>
    <span class="media-status-label">${label}</span>
    <span class="media-status-detail">${esc(detail)}</span>
  </div>`;
}

function platformLabel(url) {
  const h = (url || "").replace(/^https?:\/\//, "").split("/")[0].toLowerCase();
  if (!h) return "link";
  if (h.includes("x.com") || h.includes("twitter.com")) return "x";
  if (h.includes("reddit.com")) return "reddit";
  if (h.includes("instagram.com")) return "instagram";
  if (h.includes("pinterest.com")) return "pinterest";
  if (h.includes("youtube.com") || h.includes("youtu.be")) return "youtube";
  return h.split(".")[0] || h;
}

function handleFromUrl(url) {
  const u = url || "";
  let m = u.match(/^(?:https?:\/\/)?(?:www\.)?(?:x|twitter)\.com\/([^/?#]+)/i);
  if (m && m[1] !== "i") return "@" + m[1];
  m = u.match(/^(?:https?:\/\/)?(?:www\.)?reddit\.com\/r\/([^/?#]+)/i);
  if (m) return "r/" + m[1];
  m = u.match(/^(?:https?:\/\/)?(?:www\.)?instagram\.com\/(?:reel|reels|p|tv)\/([^/?#]+)/i);
  if (m) return "Instagram post";
  m = u.match(/^(?:https?:\/\/)?(?:www\.)?instagram\.com\/([^/?#]+)/i);
  if (m && m[1] !== "explore") return "@" + m[1];
  return "";
}

function hostNoWww(u) {
  return (u || "").toLowerCase().replace(/^https?:\/\//, "").replace(/^www\./, "").split("/")[0];
}
function isRedditItem(it) {
  return hostNoWww(it && it.url) === "reddit.com";
}

function isXItem(it) {
  const h = hostNoWww(it && it.url);
  return h === "x.com" || h === "twitter.com";
}
function xHandle(u) {
  const m = (u || "").match(/^(?:https?:\/\/)?(?:www\.)?(?:x|twitter)\.com\/([^/?#]+)/i);
  return m && m[1] && m[1] !== "i" ? m[1] : "";
}
function instagramHasMedia(it) {
  if (it.cover && it.cover.thumb) return true;
  if (Number(it.media_count || 0) > 0) return true;
  return Array.isArray(it.media) && it.media.some((media) => !media.placeholder && (media.thumb || media.file));
}

function isPrivateInstagramItem(it) {
  if (!/instagram\.com/i.test(it.url || "")) return false;
  const body = String(it.text || "").trim();
  if (/(?:account|profile|page).{0,100}private|private.{0,100}(?:account|profile|instagram)|(?:login|log in|sign up|authentication).{0,100}(?:private|profile)/i.test(body)) return true;
  if (it.job && ["queued", "working"].includes(it.job.state)) return false;
  const title = String(it.title || "").trim();
  const bareTitle = !title || /^(?:instagram(?:\.com)?|instagram post)$/i.test(title);
  const idTitle = /^[a-z0-9_-]{18,}$/i.test(title);
  return !body && !instagramHasMedia(it) && (bareTitle || idTitle);
}

function isUnavailableGfycat(it) {
  return /gfycat\.com/i.test(it.url || "");
}

function redditPostTitle(it) {
  const m = (it.url || "").match(/reddit\.com\/r\/([^/?#]+)\/comments\/[^/?#]+\/([^/?#]+)/i);
  if (!m) return "";
  let slug = "";
  try { slug = decodeURIComponent(m[2]); } catch (e) { slug = m[2]; }
  slug = slug.replace(/[-_]+/g, " ").replace(/\s+/g, " ").trim();
  if (!slug) return "r/" + m[1] + " post";
  return slug.charAt(0).toUpperCase() + slug.slice(1);
}

function displayTitle(it) {
  if (isPrivateInstagramItem(it)) return "Instagram post";
  if (isUnavailableGfycat(it)) return "Archived link";
  const reddit = redditPostTitle(it);
  if (reddit) return reddit;
  const t = (it.title || "").trim();
  if (t && !/^(?:https?:\/\/)?(?:www\.)?[a-z0-9-]+(?:\.[a-z0-9-]+)+(?:[/\s]*)$/i.test(t)) return t;
  if (isRedditItem(it)) return "reddit link";
  return linkSummary(it);
}

function realText(it) {
  const t = (it.text || it.title || "").trim();
  if (!t) return "";
  // bare-domain titles ("reddit.com") carry no info. Treat them as empty.
  if (/^(?:https?:\/\/)?(?:www\.)?[a-z0-9-]+(?:\.[a-z0-9-]+)+(?:[/\s]*)$/i.test(t)) return "";
  return t;
}

function displayText(it) {
  if (isPrivateInstagramItem(it)) return "Account private";
  if (isUnavailableGfycat(it)) return "Archived link";
  const text = cleanInstagramText(it) || realText(it);
  if (!isRedditItem(it)) return text;
  const title = (it.title || "").trim();
  // The importer copied its generated post summary into both fields.
  if (title && text && title === text) return "";
  if (/^reddit-gallery-/i.test(title)) return "";
  return text;
}

function cleanInstagramText(it) {
  const text = (it.text || "").trim();
  if (!/instagram\.com/i.test(it.url || "") || !text) return text;
  if (/(?:account|profile|page).{0,100}private|private.{0,100}(?:account|profile|instagram)|(?:login|log in|sign up|authentication).{0,100}(?:private|profile)/i.test(text)) return "Account private";
  let t = text
    .replace(/^this is an instagram social media post page\.?\s*/i, "Instagram post")
    .replace(/^an instagram social media post\.?\s*/i, "Instagram post")
    .replace(/^an instagram social media profile page\s*/i, "Instagram profile ")
    .replace(/^this is an instagram social media profile page\s*/i, "Instagram profile ")
    .replace(/^this instagram page is private\.?\s*/i, "Private Instagram page. ")
    .replace(/^this is a private instagram profile page\s*/i, "Private Instagram profile. ")
    .replace(/^this is a private instagram profile\s*/i, "Private Instagram profile. ")
    .replace(/^this page is a private instagram profile\s*/i, "Private Instagram profile. ")
    .replace(/^this instagram profile is private\.?\s*/i, "Private Instagram profile. ")
    .replace(/^this page is private\.?\s*/i, "Private Instagram page. ");
  // Private-profile saves scrape as boilerplate about follow requests. One
  // short label is all the card needs.
  return t
    .replace(/\s*\.\s*/g, ". ")
    .replace(/\.\s+([a-z])/g, " $1")
    .replace(/\s+/g, " ")
    .trim();
}

function handledLabel(it) {
  const sub = handleFromUrl(it && it.url);
  if (sub) return sub;
  return platformLabel(it && it.url);
}

/* Single source identity for a card line: for reddit -> "r/<sub>",
   for x/twitter -> "@<handle>", else the platform name. Used by the
   card-sub chip and the ce-head handle. */
function sourceHandle(it) {
  const u = (it && it.url) || "";
  const h = (u || "").replace(/^https?:\/\//, "").split("/")[0].toLowerCase();
  if (h.includes("reddit.com")) {
    const m = u.match(/reddit\.com\/r\/([^/?#]+)/i);
    return m ? "r/" + m[1] : "reddit";
  }
  if (h.includes("x.com") || h.includes("twitter.com")) {
    const handle = xHandle(u);
    if (handle) return "@" + handle;
    const author = String((it && it.author) || "").trim();
    if (author) return "@" + author.replace(/^@/, "");
    return "x";
  }
  return platformLabel(u);
}

function footLabel(it) {
  const plat = platformLabel(it.url);
  const sub = handleFromUrl(it.url);
  const sc = (it.source || "").trim();
  const scl = sc.toLowerCase();
  // Text/embed cards: footer shows the platform, header shows the handle.
  if (!sc || scl === plat) return plat;
  if (scl.startsWith(plat)) return sc;
  if (plat.startsWith(scl)) return plat;
  return plat + " · " + sc;
}

function linkSummary(it) {
  if (isUnavailableGfycat(it)) return "video unavailable";
  const text = cleanInstagramText(it) || realText(it);
  if (text) return text;
  if (isPrivateInstagramItem(it)) return "Account private";
  if (/instagram\.com/i.test(it.url || "")) return "Instagram post";
  const raw = (it.url || "").trim();
  if (!raw) return "saved link";
  try {
    const u = new URL(raw);
    const parts = u.pathname.split("/").filter(Boolean).map((p) => decodeURIComponent(p));
    const lower = parts.map((p) => p.toLowerCase());
    const ci = lower.indexOf("comments");
    const ri = lower.indexOf("r");
    if (u.hostname.toLowerCase().includes("reddit.com") && ri >= 0 && parts[ri + 1]) {
      if (ci < 0 || !parts[ci + 2]) return "r/" + parts[ri + 1] + " link";
    }
    let value = ci >= 0 && parts[ci + 2] ? parts[ci + 2] : parts[parts.length - 1];
    if (!value || /^\d+$/.test(value)) value = u.hostname.replace(/^www\./, "");
    value = value.replace(/\.(?:html?|php|aspx?)$/i, "").replace(/[-_]+/g, " ").trim();
    if (u.hostname.toLowerCase().includes("gfycat.com")) return "Archived link";
    return value || u.hostname.replace(/^www\./, "");
  } catch (e) {
    return "saved link";
  }
}

function embedHtml(it, compact) {
  const unavailable = isUnavailableGfycat(it);
  const privateItem = isPrivateInstagramItem(it);
  const problem = unavailable || privateItem;
  const text = unavailable
    ? "Archived link"
    : isRedditItem(it)
      ? (redditPostTitle(it) || (/^reddit-gallery-/i.test(it.title || "") ? (displayText(it) || "Reddit post") : displayText(it)))
      : linkSummary(it);
  const plat = platformLabel(it.url);
  const mark = problem ? "×" : KIND_GLYPH[it.kind] || BRAND_MARK;
  // Media still in flight: the pipeline job is queued/working, so the image
  // may still arrive (cover poll swaps the card when it does). Flag the
  // embed so the card reads as "processing", not as a finished text card.
  const pulling = !problem && !!it.url
    && !!it.job && (it.job.state === "queued" || it.job.state === "working");
  return `
    <div class="card-embed${compact ? " compact" : ""} plat-${escAttr(plat)}${privateItem ? " ig-private" : ""}${unavailable ? " is-unavailable" : ""}${problem ? " is-problem" : ""}${pulling ? " is-pulling" : ""}">
      <div class="ce-head"><span class="ce-mark">${mark}</span><span class="ce-handle">${esc(sourceHandle(it))}</span></div>
      ${text ? `<p class="ce-text${problem ? " ce-status" : ""}">${esc(text.slice(0, compact ? 140 : 280))}</p>` : ""}
      <div class="ce-foot">${esc(footLabel(it))}${pulling ? `<span class="ce-pull"><span class="ce-pull-dot"></span>pulling media</span>` : ""}</div>
    </div>`;
}

function cardHtml(it) {
  const ar = coverAR(it);
  const postText = displayText(it);
  const noCover = !it.cover || !it.cover.thumb;
  // No usable cover -> prefer the text embed over a dead card, whatever the kind.
  const embed = (noCover && (postText || displayTitle(it) || it.url)) ? embedHtml({ ...it, text: postText }, false) : "";
  const phStyle = it.cover && it.cover.hue
    ? `style="aspect-ratio:${ar}; background-color:${coverColor(it)}55; color:${coverColor(it)}"`
    : `style="aspect-ratio:${ar}"`;
  const unavailable = isUnavailableGfycat(it);
  const privateItem = isPrivateInstagramItem(it);
  return `
    <div class="card${unavailable ? " card-unavailable" : ""}${privateItem ? " card-private" : ""}" data-id="${it.id}" data-ar="${ar}" title="${esc(displayTitle(it) || it.url || "")}"${unavailable ? " aria-label=\"video unavailable\"" : ""}>
      ${it.open_followups ? `<span class="card-fu" title="${it.open_followups === 1 ? "1 open follow-up" : it.open_followups + " open follow-ups"}">${it.open_followups}</span>` : ""}
      ${it.cover && it.cover.thumb && (isRedditItem(it) || isXItem(it)) ? `<span class="card-sub">${esc(sourceHandle(it))}</span>` : ""}
      ${it.cover && it.cover.thumb
        ? `<div class="card-img placeholder media-loading" data-media="${esc(it.cover.thumb)}" style="aspect-ratio:${ar}"><span class="thumb-skeleton" aria-hidden="true"></span></div>`
        : embed
          ? `<div class="card-img placeholder" ${phStyle}>${embed}</div>`
          : `<div class="card-img placeholder" ${phStyle}>${previewStatusHtml(it)}</div>`}
      ${it.cover && !it.cover.thumb && it.cover.hue ? `<div class="card-hue-stripe" style="background:${coverColor(it)}"></div>` : ""}
      <div class="card-hover-actions">
        ${it.url ? `<button type="button" class="card-hover-btn card-hover-open" data-card-id="${escAttr(it.id)}" data-url="${escHtmlAttr(it.url)}" title="open link" aria-label="open link">${ic("external", "sm")}</button>` : ""}
        <button type="button" class="card-hover-btn card-hover-copy" data-card-id="${escAttr(it.id)}" title="copy" aria-label="copy">${ic("copy", "sm")}</button>
        <button type="button" class="card-hover-btn card-hover-inspire" data-card-id="${escAttr(it.id)}" title="inspire from this post" aria-label="inspire from this post">${ic("zap", "sm")}</button>
        <button type="button" class="card-hover-btn card-hover-delete" data-card-id="${escAttr(it.id)}" title="delete card" aria-label="delete card">${ic("trash", "sm")}</button>
      </div>
    </div>`;
}

/* ---------- lazy card images ----------
   Cards render a placeholder (with a spinner). Its media blob downloads
   only when the card approaches the viewport, so a page never forces the
   whole batch of images at once. */
let cardObserver = null;

function observeCardLoad(ph) {
  if (!cardObserver) {
    cardObserver = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const el = entry.target;
        if (el.dataset.loaded) continue;
        el.dataset.loaded = "1";
        cardObserver.unobserve(el);
        hydrateCard(el);
      }
    }, { rootMargin: "300px" });
  }
  cardObserver.observe(ph);
}

function clearCardObserver() {
  if (cardObserver) {
    cardObserver.disconnect();
    cardObserver = null;
  }
}

async function hydrateCards(items) {
  for (const it of items) {
    const el = $("grid").querySelector(`.card[data-id="${it.id}"]`);
    if (!el) continue;
    const ph = el.querySelector(".card-img.placeholder[data-media]");
    if (!ph || ph.dataset.loaded) continue;
    observeCardLoad(ph);
  }
  requestAnimationFrame(scheduleMasonry);
}

/* ---------- cover poll ----------
   A card whose item has no cover yet renders the text embed. While the
   pipeline job downloads media, poll those items and swap the card to a
   media skeleton as soon as a cover exists (the normal lazy loader takes
   over from there). Stops when nothing is pending or after 6 rounds. */
let coverPollTimer = 0;
let coverPollRounds = 0;

function pendingCoverCards() {
  return [...$("grid").querySelectorAll(".card[data-id]")]
    .filter((el) => {
      if (el.querySelector(".card-img[data-media]")) return false;
      return !!el.querySelector(".card-img .card-embed");
    });
}

function startCoverPoll() {
  if (coverPollTimer) return;
  coverPollRounds = 0;
  scheduleCoverPoll(800);
}

function scheduleCoverPoll(delay) {
  clearTimeout(coverPollTimer);
  coverPollTimer = setTimeout(runCoverPoll, delay);
}

async function runCoverPoll() {
  coverPollTimer = 0;
  const pending = pendingCoverCards();
  if (!pending.length || coverPollRounds >= 6) return;
  coverPollRounds += 1;
  let unresolved = 0;
  for (const el of pending) {
    const id = el.dataset.id;
    try {
      const d = await api(`/api/items/${id}`);
      const it = d.item || d;
      const stillCoverless = !it.cover || !it.cover.thumb;
      if (stillCoverless) {
        // The pipeline finished without media: this item is text-only for
        // good (e.g. a link note). Stop polling it — the embed IS the card.
        if (it.job && it.job.state === "done") continue;
        // Still pulling: make sure the embed carries the processing look
        // even if it rendered before the job state existed.
        const emb = el.querySelector(".card-img .card-embed");
        if (emb) emb.classList.add("is-pulling");
        unresolved += 1;
        continue;
      }
      el.querySelector(".card-img").outerHTML =
        `<div class="card-img placeholder media-loading" data-media="${escAttr(it.cover.thumb)}" style="aspect-ratio:${coverAR(it)}"><span class="thumb-skeleton" aria-hidden="true"></span></div>`;
      const ph = el.querySelector(".card-img[data-media]");
      if (ph) observeCardLoad(ph);
      requestAnimationFrame(scheduleMasonry);
    } catch (e) {
      unresolved += 1;
    }
  }
  if (unresolved) scheduleCoverPoll(3000);
}

async function hydrateCard(ph) {
  const name = ph.dataset.media;
  if (!name) return;
  try {
    const url = await authedBlob(name);
    const im = document.createElement("img");
    im.className = "card-img card-media-enter";
    // Keep the shimmer in the DOM while the eager image fetches.
    // Lazy loading can defer a detached image until it never appears.
    im.loading = "eager";
    im.decoding = "async";
    im.style.aspectRatio = ph.style.aspectRatio || "4/3";
    const reveal = () => {
      if (!ph.isConnected) return;
      ph.replaceWith(im);
      requestAnimationFrame(() => im.classList.add("is-ready"));
      scheduleMasonry();
    };
    im.addEventListener("load", reveal, { once: true });
    im.addEventListener("error", () => {
      if (!ph.isConnected) return;
      ph.classList.add("media-unavailable");
      const skeleton = ph.querySelector(".thumb-skeleton");
      if (skeleton) skeleton.remove();
      ph.innerHTML = '<span class="media-status-mark">×</span>';
    }, { once: true });
    im.src = url;
    if (im.complete && im.naturalWidth) reveal();
  } catch (e) {
    if (!ph.isConnected) return;
    ph.classList.add("media-unavailable");
    const skeleton = ph.querySelector(".thumb-skeleton");
    if (skeleton) skeleton.remove();
  }
}

/* pick the vertical band with the most detail (entropy proxy) for cover crops */
function smartCropY(img) {
  return new Promise((resolve) => {
    img.addEventListener("error", () => resolve({ y: 50, brightness: 255 }), { once: true });
    try {
      const S = 48, win = 24;
      const c = document.createElement("canvas"); c.width = S; c.height = S;
      const ctx = c.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(img, 0, 0, S, S);
      const d = ctx.getImageData(0, 0, S, S).data;
      const rows = [];
      let total = 0;
      for (let y = 0; y < S; y++) {
        let s1 = 0, s2 = 0;
        for (let x = 0; x < S; x++) {
          const i = (y * S + x) * 4;
          const l = 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2];
          s1 += l; s2 += l * l;
          total += l;
        }
        const m = s1 / S;
        rows.push(s2 / S - m * m);
      }
      let bestY = Math.floor(S / 2), bestV = -1;
      for (let y = 0; y + win <= S; y++) {
        let v = 0;
        for (let i = y; i < y + win; i++) v += rows[i];
        if (v > bestV) { bestV = v; bestY = y; }
      }
      resolve({ y: ((bestY + win / 2) / S) * 100, brightness: total / (S * S) });
    } catch (e) {
      resolve({ y: 50, brightness: 255 });
    }
  });
}
let masonryTimer = 0;
function scheduleMasonry() {
  if (masonryTimer) return;
  masonryTimer = requestAnimationFrame(() => {
    masonryTimer = 0;
    masonry();
  });
}
function masonry() {
  const grid = $("grid");
  const cards = [...grid.querySelectorAll(".card")];
  if (!cards.length) return;
  const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--grid-gap")) || 12;
  const padL = parseFloat(getComputedStyle(grid).paddingLeft) || 0;
  const padR = parseFloat(getComputedStyle(grid).paddingRight) || 0;
  const padT = parseFloat(getComputedStyle(grid).paddingTop) || 0;
  const width = grid.clientWidth - padL - padR;
  const target = width <= 600 ? Math.min(settings.gridSize, 150) : settings.gridSize;
  const maxCols = width <= 600 ? 2 : 6;
  const cols = Math.max(1, Math.min(maxCols, Math.floor((width + gap) / (target + gap))));
  const colW = (width - gap * (cols - 1)) / cols;
  const tops = new Array(cols).fill(0);
  for (const card of cards) {
    // The quick-note composer spans two columns so typing has room.
    const span = card.classList.contains("card-note") ? Math.min(2, cols) : 1;
    card.style.width = (colW * span + gap * (span - 1)) + "px";
    let best = 0;
    if (span === 1) {
      for (let i = 1; i < cols; i++) if (tops[i] < tops[best]) best = i;
    } else {
      // leftmost adjacent pair whose tallest column is shortest
      let bestV = Infinity;
      for (let i = 0; i + span <= cols; i++) {
        const h = Math.max(...tops.slice(i, i + span));
        if (h < bestV - 0.5) { bestV = h; best = i; }
      }
    }
    card.style.left = (padL + best * (colW + gap)) + "px";
    const baseTop = Math.max(...tops.slice(best, best + span));
    card.style.top = (padT + baseTop) + "px";
    for (let i = best; i < best + span; i++) tops[i] = baseTop + card.offsetHeight + gap;
  }
  grid.style.height = (padT + Math.max(...tops, 0)) - gap + "px";
}

/* Trash reuses the posts-tab cards, so it needs the same masonry layout.
   This mirrors masonry() but targets #trash-list and its .card children. */
function trashMasonry() {
  const grid = $("trash-list");
  if (!grid) return;
  const cards = [...grid.querySelectorAll(".card")];
  if (!cards.length) { grid.style.height = ""; return; }
  const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--grid-gap")) || 12;
  const padL = parseFloat(getComputedStyle(grid).paddingLeft) || 0;
  const padR = parseFloat(getComputedStyle(grid).paddingRight) || 0;
  const padT = parseFloat(getComputedStyle(grid).paddingTop) || 0;
  const width = grid.clientWidth - padL - padR;
  const target = width <= 600 ? Math.min(settings.gridSize, 150) : settings.gridSize;
  const maxCols = width <= 600 ? 2 : 6;
  const cols = Math.max(1, Math.min(maxCols, Math.floor((width + gap) / (target + gap))));
  const colW = (width - gap * (cols - 1)) / cols;
  const tops = new Array(cols).fill(0);
  for (const card of cards) {
    card.style.width = colW + "px";
    let best = 0;
    for (let i = 1; i < cols; i++) if (tops[i] < tops[best]) best = i;
    card.style.left = (padL + best * (colW + gap)) + "px";
    card.style.top = (padT + tops[best]) + "px";
    tops[best] += card.offsetHeight + gap;
  }
  grid.style.height = (padT + Math.max(...tops, 0)) + "px";
  _trashTops = tops; // chunked append continues from here
}
let _trashTops = null;

/* The cleanup GROUP is the masonry card now: it packs like a post card and
   its member thumbnails sit inside it in a padded grid. */
function cleanupMasonry() {
  const grid = $("cleanup-groups");
  if (!grid) return;
  const cards = [...grid.querySelectorAll(".cleanup-group")];
  if (!cards.length) { grid.style.height = ""; return; }
  const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--grid-gap")) || 12;
  const padL = parseFloat(getComputedStyle(grid).paddingLeft) || 0;
  const padR = parseFloat(getComputedStyle(grid).paddingRight) || 0;
  const padT = parseFloat(getComputedStyle(grid).paddingTop) || 0;
  const width = grid.clientWidth - padL - padR;
  const target = width <= 600 ? Math.min(settings.gridSize, 150) : settings.gridSize;
  // Cap at 6 (same as the posts grid): dense -> 6 cols, regular -> 4,
  // exploded -> 2. A tighter cap makes dense and regular land on the same
  // column count, so clicking the button through the cycle shows no change.
  const maxCols = width <= 600 ? 2 : 6;
  const cols = Math.max(1, Math.min(maxCols, Math.floor((width + gap) / (target + gap))));
  const colW = (width - gap * (cols - 1)) / cols;
  const tops = new Array(cols).fill(0);
  for (const card of cards) {
    card.style.width = colW + "px";
    let best = 0;
    for (let i = 1; i < cols; i++) if (tops[i] < tops[best]) best = i;
    card.style.left = (padL + best * (colW + gap)) + "px";
    card.style.top = (padT + tops[best]) + "px";
    tops[best] += card.offsetHeight + gap;
  }
  grid.style.height = (padT + Math.max(...tops, 0)) + "px";
}

/* The group's member grid min-column follows the density: the bigger the
   outer card, the roomier each member thumb. */
function applyCleanupGridSize() {
  document.querySelectorAll(".cleanup-group").forEach((g) => {
    const size = settings.gridSize;
    const min = size <= 150 ? 76 : size >= 380 ? 120 : 92;
    const cards = g.querySelector(".cleanup-cards");
    if (cards) cards.style.gridTemplateColumns = `repeat(auto-fill, minmax(${min}px, 1fr))`;
  });
}

function syncTopbarVar() {
  const t = document.getElementById("toolbar") || document.querySelector(".topbar");
  if (t) document.documentElement.style.setProperty("--topbar-h", t.offsetHeight + "px");
}
function onResize() {
  syncTopbarVar();
  if (!$("main").classList.contains("hidden")) scheduleMasonry();
  if (fuOpen) scheduleFuMasonry();
  if (isTrashOpen()) trashMasonry();
  if ($("main").classList.contains("cleanup-open")) { applyCleanupGridSize(); cleanupMasonry(); }
}
addEventListener("resize", onResize);

function motionAllowed() {
  return settings.animations && !(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
}

function pressTarget(e) {
  const target = e.target.closest(".card, .sim-card");
  if (!target || !target.dataset.id || target.classList.contains("card-skeleton")) return null;
  return target;
}

function clearPressedTargets() {
  document.querySelectorAll(".is-pressing").forEach((el) => el.classList.remove("is-pressing"));
}

function pulseCard(card) {
  if (!card || !motionAllowed()) return;
  card.classList.remove("is-opening");
  void card.offsetWidth;
  card.classList.add("is-opening");
  setTimeout(() => card.classList.remove("is-opening"), 240);
}

function recordInteraction(card) {
  const id = card && card.dataset.id;
  if (!id) return;
  api("/api/items/" + encodeURIComponent(id) + "/interact", { method: "POST" }).catch(() => {});
}

async function deleteItem(id) {
  if (!window.confirm("move this card to trash? you can restore it later.")) return false;
  try {
    await api("/api/items/" + id, { method: "DELETE" });
    closeDetail();
    const card = [...document.querySelectorAll("#grid .card")].find((el) => el.dataset.id === id);
    if (card) {
      card.remove();
      state.offset = Math.max(0, state.offset - 1);
    }
    document.querySelectorAll(".sim-card").forEach((el) => {
      if (el.dataset.id === id) el.remove();
    });
    try {
      hydrateMeta(await api("/api/meta"));
    } catch (metaError) {
      const count = $("meta-count");
      const match = count && count.textContent.match(/[\d,]+/);
      if (match) count.textContent = Math.max(0, parseInt(match[0].replace(/,/g, ""), 10) - 1).toLocaleString();
    }
    masonry();
    return true;
  } catch (err) {
    if (err.message !== "auth required") window.alert("delete failed: " + err.message);
    return false;
  }
}

document.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  const target = pressTarget(e);
  if (target && target.classList.contains("card-unavailable")) return;
  if (target) target.classList.add("is-pressing");
});
addEventListener("pointerup", clearPressedTargets);
addEventListener("pointercancel", clearPressedTargets);

/* ---------- bulk selection (ctrl/cmd + click) ---------- */
function toggleBulkSelect(id) {
  if (bulkSelected.has(id)) {
    bulkSelected.delete(id);
    const card = document.querySelector(`#grid .card[data-id="${id}"]`);
    if (card) card.classList.remove("selected");
  } else {
    if (bulkSelected.size >= BULK_MAX) return;
    bulkSelected.add(id);
    const card = document.querySelector(`#grid .card[data-id="${id}"]`);
    if (card) card.classList.add("selected");
  }
  setBulkBar();
}

function setBulkBar() {
  const bar = $("bulk-bar");
  if (!bar) return;
  const n = bulkSelected.size;
  bar.classList.toggle("hidden", n === 0);
  const count = $("bulk-count");
  const del = $("bulk-delete");
  const ins = $("bulk-inspire");
  if (count) count.textContent = n ? n + " selected" : "0 selected";
  if (del) del.title = del.getAttribute("data-state") === "armed"
    ? `confirm delete ${n}?`
    : `delete ${n}`;
  if (ins) ins.title = `inspire from ${n}`;
  // keep card visuals in sync with the set
  document.querySelectorAll("#grid .card[data-id]").forEach((el) => {
    el.classList.toggle("selected", bulkSelected.has(el.dataset.id));
  });
}

function clearBulkSelection() {
  bulkSelected.clear();
  bulkRangeAnchor = null;
  document.querySelectorAll("#grid .card.selected").forEach((el) => el.classList.remove("selected"));
  const del = $("bulk-delete");
  if (del) del.setAttribute("data-state", "idle");
  setBulkBar();
}

async function bulkDeleteSelected() {
  const ids = [...bulkSelected];
  if (!ids.length) return;
  const del = $("bulk-delete");
  if (!del) return;
  if (!window.confirm(`delete ${ids.length} card${ids.length === 1 ? "" : "s"}?`)) return;
  del.disabled = true;
  del.title = "deleting…";
  const removed = ids.slice(0, 100);
  try {
    const result = await api("/api/cleanup/delete", { method: "POST", body: { ids: removed } });
    const gone = new Set(removed);
    document.querySelectorAll("#grid .card[data-id]").forEach((el) => {
      if (gone.has(el.dataset.id)) el.remove();
    });
    document.querySelectorAll(".sim-card").forEach((el) => {
      if (gone.has(el.dataset.id)) el.remove();
    });
    const detailEl = $("detail");
    if (detailEl && detailEl.dataset.itemId && gone.has(detailEl.dataset.itemId)) closeDetail();
    state.offset = Math.max(0, state.offset - gone.size);
    clearBulkSelection();
    if (result && result.deleted < removed.length) {
      window.alert(`deleted ${result.deleted} of ${removed.length} — ${result.missing.length} were already gone`);
    }
    try {
      hydrateMeta(await api("/api/meta"));
    } catch (metaError) {}
    scheduleMasonry();
  } catch (err) {
    if (err.message !== "auth required") window.alert("delete failed: " + err.message);
  } finally {
    del.disabled = false;
    del.removeAttribute("data-state");
    setBulkBar();
  }
}

function bulkInspireSelected() {
  const ids = [...bulkSelected];
  if (!ids.length) return;
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  closeCleanupView();
  openInspireView();
  runInspire("", "", ids);
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && bulkSelected.size) clearBulkSelection();
});

$("bulk-clear")?.addEventListener("click", clearBulkSelection);
$("bulk-delete")?.addEventListener("click", bulkDeleteSelected);
$("bulk-inspire")?.addEventListener("click", bulkInspireSelected);
// icon-only buttons, real Material Symbols SVGs (single source: ICONS)
if ($("bulk-delete")) $("bulk-delete").innerHTML = ic("trash");
if ($("bulk-inspire")) $("bulk-inspire").innerHTML = ic("zap");
if ($("bulk-clear")) $("bulk-clear").innerHTML = ic("x");

function flashHoverButton(button, ok, message) {
  if (!button) return;
  button.classList.add(ok ? "flash-ok" : "flash-fail");
  button.classList.add("tooltip-visible");
  button.dataset.flash = message || (ok ? "copied" : "failed");
  setTimeout(() => {
    button.classList.remove("flash-ok", "flash-fail", "tooltip-visible");
    delete button.dataset.flash;
  }, 1400);
}

$("grid").addEventListener("click", (e) => {
  const hoverBtn = e.target.closest(".card-hover-btn");
  if (hoverBtn) {
    e.preventDefault();
    e.stopPropagation();
    const id = hoverBtn.dataset.cardId;
    if (!id) return;
    if (hoverBtn.classList.contains("card-hover-open")) {
      if (!openExternalUrl(hoverBtn.dataset.url)) return;
      return;
    }
    if (hoverBtn.classList.contains("card-hover-copy")) {
      copyCardContent(id).then((message) => {
        flashHoverButton(hoverBtn, true, message);
      }).catch((err) => {
        flashHoverButton(hoverBtn, false, err.message);
      });
      return;
    }
    if (hoverBtn.classList.contains("card-hover-inspire")) {
      $("sort-menu").classList.add("hidden");
      $("filters").classList.add("collapsed");
      closeCleanupView();
      openInspireView();
      runInspire("", id);
      return;
    }
    if (hoverBtn.classList.contains("card-hover-delete")) {
      deleteItem(id);
      return;
    }
  }
  if (e.button !== 0) return;
  const card = pressTarget(e);
  if (!card) return;
  if (e.ctrlKey || e.metaKey) {
    e.preventDefault();
    toggleBulkSelect(card.dataset.id);
    bulkRangeAnchor = card;
    return;
  }
  // Shift+click selects every card between the range anchor and this card
  // (same DOM order as the grid).
  if (e.shiftKey && bulkRangeAnchor) {
    e.preventDefault();
    const cards = [...document.querySelectorAll("#grid .card[data-id]")];
    const a = cards.indexOf(bulkRangeAnchor);
    const b = cards.indexOf(card);
    if (a !== -1 && b !== -1) {
      const [lo, hi] = a < b ? [a, b] : [b, a];
      for (let i = lo; i <= hi; i++) {
        const id = cards[i].dataset.id;
        if (!bulkSelected.has(id) && bulkSelected.size >= BULK_MAX) break;
        bulkSelected.add(id);
      }
      setBulkBar();
    }
    bulkRangeAnchor = card;
    return;
  }
  if (card.classList.contains("card-unavailable")) return;
  pulseCard(card);
  recordInteraction(card);
  openDetail(card.dataset.id);
});

/* ---------- detail ---------- */
const DETAIL_MOTION_MS = 220;
let explainerWatchToken = 0;

function closeDetail() {
  const overlay = $("detail");
  if (!overlay || overlay.classList.contains("hidden") || overlay.classList.contains("is-closing")) return;
  overlay.classList.remove("is-opening");
  if (!motionAllowed()) {
    overlay.classList.remove("is-closing");
    overlay.classList.add("hidden");
    return;
  }
  overlay.classList.add("is-closing");
  setTimeout(() => {
    if (!overlay.classList.contains("is-closing")) return;
    overlay.classList.add("hidden");
    overlay.classList.remove("is-closing");
  }, DETAIL_MOTION_MS);
}

async function openDetail(id, options = {}) {
  const overlay = $("detail");
  overlay.dataset.itemId = id;
  overlay.dataset.trash = options.trash ? "1" : "";
  const wasHidden = overlay.classList.contains("hidden");
  overlay.classList.remove("hidden", "is-closing");
  if (wasHidden && motionAllowed()) {
    overlay.classList.remove("is-opening");
    void overlay.offsetWidth;
    overlay.classList.add("is-opening");
    setTimeout(() => overlay.classList.remove("is-opening"), DETAIL_MOTION_MS);
  } else {
    overlay.classList.remove("is-opening");
  }
  $("detail-body").innerHTML = '<p class="detail-text loading-copy"><span class="spin ui-mark-wrap">' + BRAND_MARK + '</span> loading…</p>';
  try {
    const it = await api("/api/items/" + id);
    renderDetail(it, options);
  } catch (e) {
    $("detail-body").innerHTML = '<p class="error">' + esc(e.message) + "</p>";
  }
}

function rowMarkup(key, label, extra, body, openOverride = null) {
  const open = openOverride == null ? !!settings.rows[key] : openOverride;
  return `<button class="row-toggle${open ? " open" : ""}" type="button" data-row="${key}" aria-expanded="${open}"><span>${label}</span>${extra || ""}<span class="row-chev">${ic("chev-r", "sm")}</span></button>` +
         `<div class="row-body${open ? "" : " hidden"}" data-body="${key}">${body}</div>`;
}

const SIMILAR_PAGE_SIZE = 48;

function similarCardHtml(s, index) {
  const simText = displayText(s);
  const simTitle = displayTitle(s);
  const isUsableTitle = (item) => {
    const t = (item.title || "").trim();
    return !!t && t.toLowerCase() !== "reddit.com" && t.toLowerCase() !== "gfycat.com";
  };
  const hasUsablePostText = !!simText && (!isRedditItem(s) || !((s.title || "").trim() === simText));
  const simEmbed = ((s.kind === "text" || s.kind === "link") &&
    (!s.cover || !s.cover.thumb || !hasUsablePostText || !isUsableTitle(s)) &&
    (hasUsablePostText || simTitle || s.url))
    ? embedHtml({ ...s, text: hasUsablePostText ? simText : "" }, true)
    : "";
  const unavailable = isUnavailableGfycat(s);
  const privateItem = isPrivateInstagramItem(s);
  return `
    <div class="sim-card sim-card-enter${unavailable ? " sim-card-unavailable" : ""}${privateItem ? " sim-card-private" : ""}" data-id="${s.id}" style="--sim-delay:${Math.min(420, index * 35)}ms">
      ${simEmbed
        ? `<div class="card-img placeholder" style="background:${coverColor(s)}2e">${simEmbed}</div>`
        : s.cover && s.cover.thumb
          ? `<div class="card-img placeholder sim-card-loading" data-ph="${esc(s.cover.thumb)}"><span class="sim-thumb-skeleton" aria-hidden="true"></span></div>`
          : `<div class="card-img placeholder" style="background:${coverColor(s)}44;color:${coverColor(s)}">${previewStatusHtml(s, true)}</div>`}
    </div>`;
}

function bindSimilarCards(root) {
  if (!root) return;
  root.querySelectorAll(".sim-card").forEach((sc) => {
    if (sc.dataset.bound) return;
    sc.dataset.bound = "1";
    sc.addEventListener("click", () => {
      if (sc.classList.contains("sim-card-unavailable")) return;
      pulseCard(sc);
      recordInteraction(sc);
      openDetail(sc.dataset.id);
    });
  });
}

function hydrateSimilarMedia(root, items, detailItem) {
  if (!root) return;
  const itemById = new Map((items || []).map((item) => [item.id, item]));
  for (const s of root.querySelectorAll(".sim-card .card-img[data-ph]")) {
    const card = s.closest(".sim-card");
    const simItem = itemById.get(card && card.dataset.id);
    authedBlob(s.dataset.ph).then((u) => {
      const im = document.createElement("img");
      im.loading = "eager";
      im.className = "card-img sim-card-media-enter";
      im.style.aspectRatio = "1";
      const showFallback = () => {
        if (!s.isConnected) return;
        s.innerHTML = previewStatusHtml(simItem || detailItem, true);
        s.classList.add("media-unavailable");
      };
      const reveal = () => {
        if (!s.isConnected) return;
        s.replaceWith(im);
        requestAnimationFrame(() => im.classList.add("is-ready"));
      };
      im.addEventListener("load", reveal, { once: true });
      im.addEventListener("error", showFallback, { once: true });
      im.src = u;
      if (im.complete && im.naturalWidth) reveal();
      const settle = () => {
        if (im.dataset.settled) return;
        im.dataset.settled = "1";
        smartCropY(im).then(({ y, brightness }) => {
          if (brightness < 28) {
            const simCard = im.closest(".sim-card");
            if (simItem && (simItem.text || simItem.title)) {
              const holder = document.createElement("div");
              holder.className = "card-img placeholder";
              holder.style.backgroundColor = coverColor(simItem) + "2e";
              holder.innerHTML = embedHtml(simItem, true);
              im.replaceWith(holder);
              if (simCard) simCard.classList.add("has-embed");
              return;
            }
          }
          im.style.objectPosition = "50% " + y.toFixed(0) + "%";
        });
      };
      const whenLoaded = (cb) => (im.complete ? cb() : im.addEventListener("load", cb, { once: true }));
      if (simItem && simItem.kind === "video") whenLoaded(() => setTimeout(settle, 1000));
      else whenLoaded(settle);
    }).catch(() => {
      if (!s.isConnected) return;
      s.innerHTML = previewStatusHtml(simItem || detailItem, true);
      s.classList.add("media-unavailable");
    });
  }
}

function renderDetail(it, options = {}) {
  const trashMode = !!options.trash;
  const detailTitle = /reddit\.com\/gallery\//i.test(it.url || "") ? "Reddit post" : displayTitle(it);
  const fuCount = Number(it.open_followups || 0);
  const fuChip = fuCount
    ? `<button type="button" class="detail-fu-chip" title="open follow-ups|aria-label="open follow-ups"><span class="fu-dot"></span>${fuCount} follow-up${fuCount === 1 ? "" : "s"}</button>`
    : "";
  const { media, mediaFallback, multiMedia, viewableCount } = detailMediaHtml(it);
  const transcriptText = (it.transcripts || []).map((t) => t.text).filter(Boolean).join("\n\n");
  const primaryText = displayText(it) || transcriptText;
  const isMyMindItem = it.is_mymind === true || !!it.mymind_id || it.source === "mymind";
  const explHtml = detailExplainerHtml(it, primaryText, isMyMindItem);
  const simItems = it.similar || [];
  const sim = simItems.map(similarCardHtml).join("");
  const similarTotal = Number(it.similar_total || simItems.length);
  const similarHasMore = it.similar_has_more !== false;

  $("detail-body").innerHTML = `
    <div class="detail-layout${settings.sidebarOpen ? " panel-open" : ""}">
      <div class="detail-media-col${multiMedia ? " is-carousel" : ""}">
        ${multiMedia
          ? `<div class="media-carousel" id="media-carousel">
               <div class="media-track">${media}</div>
               <button type="button" class="media-nav media-nav-prev" title="previous" aria-label="previous picture">${ic("chev-l", "sm")}</button>
               <button type="button" class="media-nav media-nav-next" title="next" aria-label="next picture">${ic("chev-r", "sm")}</button>
               <span class="media-counter">1 / ${viewableCount}</span>
             </div>`
          : (media || mediaFallback)}
      </div>
      <div class="detail-minibar">
        <span class="mini-title">${esc(detailTitle || "untitled")}</span>
        <div class="detail-actions">
          ${trashMode ? "" : `<button class="icbtn" id="btn-inspire-seed-mini" title="use this post as an inspire seed" aria-label="use this post as an inspire seed">${ic("zap", "sm")}</button>`}
          <button class="icbtn" id="btn-panel-open" title="expand panel">${ic("panel-open", "sm")}</button>
          <button class="icbtn js-close" title="close">${ic("x", "sm")}</button>
        </div>
      </div>
      <div class="detail-panel">
        <div class="panel-head">
          <h2 class="detail-title">${esc(detailTitle || "untitled")}${fuChip}</h2>
          <div class="detail-actions">
            ${trashMode ? "" : `<button class="icbtn" id="btn-inspire-seed" title="use this post as an inspire seed" aria-label="use this post as an inspire seed">${ic("zap", "sm")}</button>`}
            ${trashMode ? "" : `<button class="icbtn" id="btn-re-expl" title="refresh post — re-pull media, colors, OCR, explainer, follow-ups">${ic("refresh", "sm")}</button>`}
            <button class="icbtn danger" id="btn-delete" title="delete">${ic("trash", "sm")}</button>
            <button class="icbtn" id="btn-open-url" title="open original" ${it.url ? "" : "disabled"}>${ic("external", "sm")}</button>
            <button class="icbtn" id="btn-panel-close" title="collapse panel">${ic("panel-close", "sm")}</button>
            <button class="icbtn js-close" title="close">${ic("x", "sm")}</button>
          </div>
        </div>
        <div class="panel-rows">
          ${rowMarkup("meta", "meta", "", `<p class="detail-sub">${esc(it.source || "")}${(isRedditItem(it) || isXItem(it)) ? " · " + esc(sourceHandle(it)) : ""} · ${esc(it.kind || "")} · ${it.saved_at ? esc(it.saved_at) : ""}</p>`)}
          ${it.note ? rowMarkup("note", "note", "", `<p class="detail-text">${esc(it.note)}</p>`) : ""}
          ${["pending", "working"].includes(it.explanation_status) ? explainerPendingMarkup() : rowMarkup("explainer", "explainer", "", explHtml, options.explainerOpen)}
          ${sim ? rowMarkup("similar", "similar", `<span class="row-count">${similarTotal}</span>`, `<div class="sim-scroll"><div class="sim-grid">${sim}</div><div class="sim-load-state" role="status"></div></div>`) : ""}
        </div>
      </div>
    </div>`;

  bindDetailEvents(it, { simItems, simTotal: similarTotal, similarHasMore, explainerOpen: options.explainerOpen, trashMode });
}

function detailMediaHtml(it) {
  const mediaList = it.media || [];
  const isViewable = (m) => m.kind !== "audio" && m.kind !== "file";
  const viewableCount = mediaList.filter(isViewable).length;
  const multiMedia = viewableCount > 1;
  const media = mediaList.map((m) => {
    const src = m.thumb || m.file || "";
    if (m.kind === "audio") {
      return `<div class="detail-media detail-audio"><span class="detail-file-kind">audio</span><audio controls preload="metadata" data-src="${esc(src)}"></audio><span class="detail-file-name">${esc(m.original_name || "audio attachment")}</span></div>`;
    }
    if (m.kind === "file") {
      return `<div class="detail-media detail-file"><span class="detail-file-kind">file</span><span class="detail-file-name">${esc(m.original_name || "file attachment")}</span><a class="detail-file-download" data-src="${esc(src)}" download="${esc(m.original_name || "attachment")}">download</a></div>`;
    }
    return src ? `<img class="detail-media" data-src="${esc(src)}">` : "";
  }).join("");
  const mediaFallback = (it.kind === "text" || it.kind === "link")
    ? `<div class="detail-media placeholder detail-link-card">${isRedditItem(it) && /^reddit-gallery-/i.test(it.title || "") ? embedHtml({ ...it, title: "Reddit post", text: displayText(it) }, false) : embedHtml(it, false)}</div>`
    : `<div class="detail-media placeholder detail-unavailable">${previewStatusHtml(it)}</div>`;
  return { media, mediaFallback, multiMedia, viewableCount };
}

function paletteSwatchesHtml(it) {
  const palette = Array.isArray(it.cover?.palette) ? it.cover.palette.slice(0, 6) : [];
  if (!palette.length) return "";
  return `<div class="explainer-palette"><span class="explainer-context-label">colors</span><div class="swatch-row">${palette.map((color) => {
    const hex = String(color.hex || "").toLowerCase();
    const rgb = hexToRgb(hex);
    const hsv = rgbToHsvArr(rgb);
    const numeric = rgb ? `rgb(${rgb.join(", ")}) · hsv(${hsv.join(", ")})` : "";
    return `<button type="button" class="swatch" style="background:${esc(hex)}" data-hex="${esc(hex)}" data-copy="${esc(hex)}" title="" aria-label="color ${esc(hex)}"><span class="swatch-code">#${esc(hex.replace(/^#/, ""))}<span class="swatch-num">${esc(numeric)}</span></span></button>`;
  }).join("")}</div></div>`;
}

function bindPaletteSwatches(container) {
  for (const sw of container.querySelectorAll(".swatch")) {
    sw.addEventListener("mouseenter", () => {
      sw.classList.remove("swatch-data-left", "swatch-data-right");
      const panel = container.closest(".detail-card") || container;
      const panelRect = panel.getBoundingClientRect();
      const swRect = sw.getBoundingClientRect();
      const tooltipW = Math.min(320, swRect.width + 220);
      if (tooltipW > swRect.left - panelRect.left) sw.classList.add("swatch-data-left");
      else if (swRect.right + 220 > panelRect.right) sw.classList.add("swatch-data-right");
    });
    sw.addEventListener("click", async (e) => {
      e.stopPropagation();
      try {
        await copyText(sw.dataset.copy);
        const prev = sw.getAttribute("aria-label");
        sw.classList.add("copied");
        sw.setAttribute("aria-label", "copied");
        setTimeout(() => {
          sw.classList.remove("copied");
          sw.setAttribute("aria-label", prev);
        }, 900);
      } catch (err) {
        /* copy unavailable — do nothing except keep the swatch usable */
      }
    });
  }
}

function detailExplainerHtml(it, primaryText, isMyMindItem) {
  const expl = it.explanation;
  const explainerTags = (it.tags || []).map((tag) => `<span class="chip-tag">${esc(tag)}</span>`).join("");
  const explainerTagsMarkup = explainerTags
    ? `<div class="explainer-tags"><span class="explainer-context-label">tags</span><div class="tag-row">${explainerTags}</div></div>`
    : "";
  const paletteMarkup = paletteSwatchesHtml(it);
  const explainerText = primaryText
    ? section(isMyMindItem ? "saved text" : "text", esc(primaryText))
    : "";
  const explSections = expl ? [
    expl.summary && section("summary", esc(expl.summary)),
    expl.visuals && section("visuals", esc(Array.isArray(expl.visuals) ? expl.visuals.join(", ") : expl.visuals)),
    expl.mood && section("mood", esc(expl.mood)),
    expl.style_dna && section("style dna", esc(Array.isArray(expl.style_dna) ? expl.style_dna.join(" · ") : expl.style_dna)),
    expl.why_it_fits && section("why it fits you", esc(expl.why_it_fits)),
    expl.color_words && section("color words", esc(expl.color_words)),
    expl.video_note && section("video note", esc(expl.video_note)),
  ].filter(Boolean) : (it.text ? [] : [`<p class="detail-text">explainer not generated.</p>`]);
  return [explainerTagsMarkup, paletteMarkup, ...explSections, explainerText].filter(Boolean).join("");
}

function bindDetailEvents(it, opts) {
  const { simItems, simTotal, similarHasMore, trashMode } = opts;
  const body = $("detail-body");
  for (const m of body.querySelectorAll("[data-src]")) {
    authedBlob(m.dataset.src).then((u) => {
      if (m.tagName === "A") m.href = u;
      else m.src = u;
      m.addEventListener("error", () => {
        if (m.tagName !== "IMG") return;
        const fallback = document.createElement("div");
        fallback.className = "detail-media placeholder detail-unavailable";
        fallback.innerHTML = previewStatusHtml(it);
        m.replaceWith(fallback);
      }, { once: true });
    }).catch(() => {
      if (m.tagName === "A") {
        m.textContent = "download unavailable";
        m.removeAttribute("href");
        return;
      }
      if (m.tagName === "AUDIO") {
        m.replaceWith(document.createTextNode("audio unavailable"));
        return;
      }
      const fallback = document.createElement("div");
      fallback.className = "detail-media placeholder detail-unavailable";
      fallback.innerHTML = previewStatusHtml(it);
      m.replaceWith(fallback);
    });
  }
  const simScroll = body.querySelector(".sim-scroll");
  bindPaletteSwatches(body);
  const carousel = body.querySelector("#media-carousel");
  if (carousel) {
    const track = carousel.querySelector(".media-track");
    const slides = [...track.querySelectorAll(".detail-media")];
    const prev = carousel.querySelector(".media-nav-prev");
    const next = carousel.querySelector(".media-nav-next");
    const counter = carousel.querySelector(".media-counter");
    const slideWidth = () => (slides[0] ? slides[0].getBoundingClientRect().width : track.clientWidth);
    const current = () => Math.round(track.scrollLeft / Math.max(1, slideWidth()));
    const update = () => {
      const idx = Math.min(slides.length, Math.max(0, current() + 1));
      if (counter) counter.textContent = `${idx} / ${slides.length}`;
      const maxScroll = track.scrollWidth - track.clientWidth - 4;
      prev.classList.toggle("is-disabled", track.scrollLeft <= 4);
      next.classList.toggle("is-disabled", track.scrollLeft >= maxScroll);
    };
    const go = (dir) => {
      const target = Math.min(slides.length - 1, Math.max(0, current() + dir));
      track.scrollTo({ left: target * slideWidth(), behavior: motionAllowed() ? "smooth" : "auto" });
    };
    prev.addEventListener("click", () => go(-1));
    next.addEventListener("click", () => go(1));
    track.addEventListener("scroll", () => requestAnimationFrame(update), { passive: true });
    update();
  }
  const simGrid = body.querySelector(".sim-grid");
  const simLoadState = body.querySelector(".sim-load-state");
  let simOffset = simItems.length;
  let simLoading = false;
  let simDone = !similarHasMore;
  const updateSimilarLoadState = () => {
    if (!simLoadState) return;
    if (simDone) {
      simLoadState.textContent = simTotal ? `all ${simTotal.toLocaleString()} similar shown` : "no similar items";
      simLoadState.classList.add("done");
      return;
    }
    simLoadState.classList.remove("done");
    simLoadState.innerHTML = `<span>showing ${simOffset.toLocaleString()} of ${simTotal.toLocaleString()}</span><button type="button" class="sim-load-btn">show more</button>`;
    simLoadState.querySelector(".sim-load-btn").addEventListener("click", loadMoreSimilar);
  };
  async function loadMoreSimilar() {
    if (simLoading || simDone || !simGrid) return;
    simLoading = true;
    if (simLoadState) simLoadState.innerHTML = "loading more similar…";
    try {
      const params = new URLSearchParams({ offset: simOffset, limit: SIMILAR_PAGE_SIZE });
      const data = await api("/api/items/" + it.id + "/similar?" + params);
      const items = data.items || [];
      const start = simOffset;
      simGrid.insertAdjacentHTML("beforeend", items.map((item, index) => similarCardHtml(item, start + index)).join(""));
      simOffset += items.length;
      simDone = !data.has_more || !items.length;
      bindSimilarCards(simGrid);
      hydrateSimilarMedia(simGrid, items, it);
    } catch (err) {
      if (simLoadState) simLoadState.textContent = "could not load more similar";
    } finally {
      simLoading = false;
      updateSimilarLoadState();
    }
  }
  const maybeLoadSimilar = () => {
    if (!simScroll || simLoading || simDone) return;
    if (simScroll.scrollTop + simScroll.clientHeight >= simScroll.scrollHeight - 360) loadMoreSimilar();
  };
  hydrateSimilarMedia(body, simItems, it);
  bindSimilarCards(body);
  updateSimilarLoadState();
  if (simScroll) simScroll.addEventListener("scroll", maybeLoadSimilar, { passive: true });
  requestAnimationFrame(maybeLoadSimilar);
  const layout = body.querySelector(".detail-layout");
  const po = body.querySelector("#btn-panel-open");
  if (po) po.addEventListener("click", () => layout.classList.add("panel-open"));
  const pc = body.querySelector("#btn-panel-close");
  if (pc) pc.addEventListener("click", () => layout.classList.remove("panel-open"));
  const mc = body.querySelectorAll(".js-close");
  mc.forEach((b) => b.addEventListener("click", closeDetail));
  body.querySelectorAll(".row-toggle").forEach((rw) => rw.addEventListener("click", () => {
    const open = rw.classList.toggle("open");
    rw.setAttribute("aria-expanded", String(open));
    const rowBody = body.querySelector(`[data-body="${rw.dataset.row}"]`);
    if (rowBody) rowBody.classList.toggle("hidden", !open);
    if (open && rw.dataset.row === "similar") requestAnimationFrame(maybeLoadSimilar);
  }));
  const re = body.querySelector("#btn-re-expl");
  if (re) re.addEventListener("click", async () => {
    if (re.dataset.pending === "true") return;
    const runToken = ++explainerWatchToken;
    const explainerToggle = body.querySelector('.row-toggle[data-row="explainer"]');
    const explainerOpen = !!explainerToggle?.classList.contains("open");
    re.dataset.pending = "true";
    re.disabled = true;
    setExplainerPending();
    try {
      await api("/api/items/" + it.id + "/refresh", { method: "POST" });
      await waitForExplainer(it.id, runToken, explainerOpen);
    } catch (err) {
      if (runToken !== explainerWatchToken) return;
      showExplainerError(err.message);
    }
  });
  const startSeedInspire = () => {
    closeDetail();
    $("sort-menu").classList.add("hidden");
    $("filters").classList.add("collapsed");
    closeCleanupView();
    openInspireView();
    runInspire("", it.id);
  };
  const seedButtons = body.querySelectorAll("#btn-inspire-seed, #btn-inspire-seed-mini");
  seedButtons.forEach((button) => button.addEventListener("click", startSeedInspire));
  const del = body.querySelector("#btn-delete");
  if (del) del.addEventListener("click", () => deleteItem(it.id));
  const fuChipEl = body.querySelector(".detail-fu-chip");
  if (fuChipEl) {
    fuChipEl.addEventListener("click", (e) => {
      e.stopPropagation();
      closeDetail();
      openFollowupsView();
    });
  }
  const openUrl = body.querySelector("#btn-open-url");
  if (openUrl && !isUnavailableGfycat(it)) openUrl.addEventListener("click", () => window.open(it.url, "_blank"));
  if (openUrl && isUnavailableGfycat(it)) {
    openUrl.disabled = true;
    openUrl.title = "source unavailable";
  }
  if (it.source === "note" && ((it.job && ["queued", "working"].includes(it.job.state)) ||
      ["pending", "working"].includes(it.explanation_status))) {
    setTimeout(() => {
      if (!$("detail").classList.contains("hidden")) openDetail(it.id);
    }, 900);
  }
}

function explainerPendingMarkup() {
  return `<div class="explainer-pending" data-explainer-pending="true" role="status" aria-live="polite" aria-busy="true">
    <div class="explainer-pending-head"><span>explainer</span><span class="explainer-pending-status">regenerating…</span></div>
    <div class="explainer-skeleton" aria-hidden="true">
      <span class="explainer-skeleton-line explainer-skeleton-line-wide"></span>
      <span class="explainer-skeleton-line"></span>
      <span class="explainer-skeleton-line explainer-skeleton-line-short"></span>
    </div>
  </div>`;
}

function setExplainerPending() {
  const rows = $("detail-body").querySelector(".panel-rows");
  if (!rows) return;
  const pending = rows.querySelector("[data-explainer-pending]");
  if (pending) {
    pending.outerHTML = explainerPendingMarkup();
    return;
  }
  const toggle = rows.querySelector('.row-toggle[data-row="explainer"]');
  const body = rows.querySelector('.row-body[data-body="explainer"]');
  if (!toggle) return;
  body?.remove();
  toggle.outerHTML = explainerPendingMarkup();
}

function showExplainerError(message) {
  const pending = $("detail-body").querySelector("[data-explainer-pending]");
  if (!pending) return;
  pending.classList.add("error");
  pending.setAttribute("aria-busy", "false");
  const status = pending.querySelector(".explainer-pending-status");
  if (status) status.textContent = "regeneration failed: " + message;
  pending.querySelectorAll(".explainer-skeleton-line").forEach((line) => line.remove());
  const button = $("detail-body").querySelector("#btn-re-expl");
  if (button) {
    button.disabled = false;
    delete button.dataset.pending;
  }
}

async function waitForExplainer(id, runToken, explainerOpen) {
  const startedAt = Date.now();
  while (runToken === explainerWatchToken) {
    await new Promise((resolve) => setTimeout(resolve, 900));
    const detail = $("detail");
    if (runToken !== explainerWatchToken || detail.classList.contains("hidden") || detail.dataset.itemId !== id) return;
    const latest = await api("/api/items/" + id);
    if (!["done", "failed"].includes(latest.explanation_status)) continue;
    // The refresh endpoint returns before the background thread queues the
    // explainer; ignore a stale 'done' from before the refresh took effect.
    if (["done", "failed"].includes(latest.explanation_status) && Date.now() - startedAt < 8000) continue;
    if (runToken !== explainerWatchToken || detail.classList.contains("hidden") || detail.dataset.itemId !== id) return;
    renderDetail(latest, { explainerOpen });
    return;
  }
}

function section(h, body) {
  return `<div class="section"><h3>${h}</h3><p class="detail-text">${body}</p></div>`;
}

$("detail-close").addEventListener("click", closeDetail);
$("detail").addEventListener("click", (e) => { if (e.target === $("detail")) closeDetail(); });

/* ---------- inspire (bolt runs on the search-box text) ---------- */
function closeInspireView() {
  $("main").classList.remove("inspire-open");
  $("inspire").classList.add("hidden");
}
function openInspireView() {
  collapseHoverRail();
  closeDetail();
  closeFollowupsView(); // full-page views are exclusive
  closeTrashView();
  closeGroupsView();
  $("inspire").classList.remove("hidden");
  $("main").classList.add("inspire-open");
  pushView("inspire");
}
function resetInspireControls() {
  const tools = $("inspire-tools");
  const actions = $("inspire-tool-actions");
  const copy = $("inspire-copy");
  if (tools && actions) tools.appendChild(actions);
  if (tools) tools.classList.remove("hidden");
  if (copy) {
    copy.classList.add("hidden");
    copy.classList.remove("copied");
    copy.title = "copy direction";
  }
}

$("inspire-close").addEventListener("click", closeInspireView);

// brief bar: one input, one go, one "suggest something"
const briefAsk = () => {
  const q = $("brief-q");
  runInspire(q ? q.value.trim() : "");
};
$("brief-go").addEventListener("click", briefAsk);
$("brief-q").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    briefAsk();
  }
});
$("brief-suggest").addEventListener("click", () => {
  const q = $("brief-q");
  if (q) q.value = "";
  runInspire("");
});

$("btn-inspire").addEventListener("click", (e) => {
  e.preventDefault();
  $("sort-menu").classList.add("hidden");
  $("filters").classList.add("collapsed");
  closeCleanupView();
  $("settings").classList.add("hidden"); // bolt from settings: leave the shell first
  openInspireView();
  runInspire($("search-q").value.trim());
});

function runInspire(about, seedId = "", seedIds = []) {
  resetInspireControls();
  resetInspirePreview();
  const askBox = $("brief-q");
  if (askBox && typeof about === "string") askBox.value = about;
  const status = $("insp-status");
  status.className = "import-status";
  const nSeeds = seedIds.length || (seedId ? 1 : 0);
  status.textContent = nSeeds > 1
    ? `using ${nSeeds} posts as visual seeds…`
    : seedId
      ? "using this post as the visual seed…"
      : about ? "building a brief for: " + about : "picking an idea from the vault…";
  $("inspire-results").classList.add("hidden");
  const body = { mode: "brief", project: "mixed", platform: "any", about, seed_id: seedId, seed_ids: seedIds };
  api("/api/inspire", { method: "POST", body })
    .then((started) => watchInspireJob(started.job_id, status))
    .catch((err) => {
      if (err.message === "auth required") return;
      status.className = "import-status error";
      status.textContent = "inspire failed: " + err.message;
    });
}

async function watchInspireJob(jobId, status) {
  const phase = { queued: "sampling your vault…", working: "writing the brief…" };
  while (true) {
    await new Promise((resolve) => setTimeout(resolve, 1300));
    let job;
    try {
      job = await api("/api/inspire/" + encodeURIComponent(jobId));
    } catch (err) {
      if (err.message === "auth required") return;
      status.textContent = "polling failed: " + err.message;
      continue;
    }
    if (!job || job.state === "missing") {
      status.className = "import-status error";
      status.textContent = "inspire job vanished";
      return;
    }
    if (job.state === "error") {
      status.className = "import-status error";
      status.textContent = "inspire failed: " + (job.error || "unknown");
      return;
    }
    if (job.state === "done") {
      status.className = "import-status";
      status.textContent = "";
      const result = job.result || {};
      if (result.mode === "brief") {
        renderBrief(result);
        return;
      }
      renderInspireResult(result);
      renderInspirePreviewComposer(result, jobId);
      const exportBtn = $("inspire-export");
      if (exportBtn) exportBtn.classList.remove("hidden");
      return;
    }
    status.classList.remove("error");
    status.textContent = phase[job.state] || "working…";
  }
}

function resolveInspRef(token, candidates) {
  if (token == null) return "";
  const t = String(token);
  const m = t.match(/^r(\d{1,2})$/i);
  const idx = m ? Number(m[1]) : -1;
  if (idx >= 0 && idx < candidates.length) return candidates[idx].id;
  return candidates.some((c) => c.id === t) ? t : "";
}

function inspRefChips(refs, candidates) {
  const values = Array.isArray(refs) ? refs : refs == null ? [] : [refs];
  const seen = new Set();
  return values.map((ref) => {
    const id = resolveInspRef(ref, candidates);
    if (!id || seen.has(id)) return "";
    seen.add(id);
    const raw = String(ref == null ? "" : ref);
    const label = /^r\d{1,2}$/i.test(raw) ? raw : "ref " + id.slice(0, 6);
    return `<button type="button" class="chip inspire-ref-chip" data-ref-id="${escAttr(id)}" title="open this saved item">${esc(label)}</button>`;
  }).join("");
}

/* Google Fonts preview helpers */
const SYSTEM_FONTS = new Set(["system-ui", "sans-serif", "serif", "monospace", "ui-sans-serif", "ui-serif", "ui-monospace", "arial", "helvetica", "georgia", "times new roman", "courier new", "times", "courier", "impact", "verdana"]);

function cssFamilyString(family) {
  const clean = String(family || "").trim().replace(/["'\\]/g, "");
  return clean ? "'" + clean + "', system-ui, sans-serif" : "system-ui, sans-serif";
}

function canLoadGoogleFont(family) {
  const f = String(family || "").trim().toLowerCase();
  if (!/^[a-z0-9 .&()-]+$/i.test(String(family || "").trim())) return false;
  return !SYSTEM_FONTS.has(f) && f.length > 1 && f.length < 80;
}

function loadGoogleFont(family) {
  const clean = String(family || "").trim();
  if (!canLoadGoogleFont(clean)) return;
  const href = "https://fonts.googleapis.com/css2?family=" + encodeURIComponent(clean) + ":wght@400;600;800&display=swap";
  if (document.querySelector(`link[href="${href}"]`)) return;
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = href;
  document.head.appendChild(link);
}

function familySpecimenUrl(family) {
  const clean = String(family || "").trim();
  if (!clean) return "";
  return "https://fonts.google.com/specimen/" + clean.replace(/\s+/g, "+");
}

/* ---------- inspire result rendering ---------- */
function buildInspireCopy(direction) {
  const lines = [];
  if (direction.title) lines.push("DIRECTION: " + direction.title.toUpperCase() + "\n");
  if (direction.summary) lines.push(direction.summary + "\n");
  const palette = Array.isArray(direction.palette) ? direction.palette : [];
  if (palette.length) {
    lines.push("palette");
    palette.forEach((p) => lines.push(`- ${p.hex || ""}${p.name ? " " + p.name : ""}${p.role ? " / " + p.role : ""}`));
  }
  const typo = direction.typography || {};
  if (typo.direction) lines.push("\ntypography\n" + typo.direction);
  (typo.families || []).forEach((f) => lines.push(`- ${f.family || ""} (${f.role || "body"}): ${f.why || ""}`));
  const materials = direction.materials || direction.material_language || null;
  if (materials && (materials.name || materials.concept || (materials.facets || []).length)) {
    lines.push("\nmaterials — " + (materials.name || "one system"));
    if (materials.concept) lines.push(materials.concept);
    (materials.facets || []).forEach((x) => lines.push(`- ${x.name || ""}: ${x.what || ""}${x.ref ? " (" + x.ref + ")" : ""}`));
  } else {
    const treatments = Array.isArray(direction.treatments) ? direction.treatments : [];
    if (treatments.length) {
      lines.push("\ntreatments");
      treatments.forEach((x) => lines.push(`- ${x.name || ""}: ${x.what || ""}`));
    }
  }
  const tags = Array.isArray(direction.tags) ? direction.tags : Array.isArray(direction.mood) ? direction.mood : [];
  if (tags.length) lines.push("\ntags: " + tags.join(", "));
  return lines.join("\n");
}

function contrastText(hex) {
  const rgb = hexToRgb(String(hex || "").replace(/^#/, ""));
  if (!rgb) return "#17121f";
  const [r, g, b] = rgb.map((v) => v / 255);
  const lin = [r, g, b].map((c) => (c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4)));
  const L = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
  return L > 0.33 ? "#17121f" : "#f2eefa";
}

/* ---------- build brief: one thing to make, from saved taste ---------- */

function renderBrief(result) {
  const host = $("inspire-results");
  if (!host) return;
  const b = result.brief || {};
  const refs = Array.isArray(result.refs) ? result.refs : [];
  const pal = Array.isArray(b.palette) ? b.palette : [];
  const themes = Array.isArray(b.themes) ? b.themes : [];
  const build = b.build && typeof b.build === "object" ? b.build : {};
  const parts = Array.isArray(build.parts) ? build.parts : [];
  const make = Array.isArray(b.make) ? b.make : [];
  const steal = Array.isArray(b.steal) ? b.steal : [];
  const ts = b.type_system && typeof b.type_system === "object" ? b.type_system : null;
  const fams = ts && Array.isArray(ts.families) ? ts.families : [];
  fams.forEach((f) => loadGoogleFont(f.family));

  const themesHtml = themes.length
    ? `<h4 class="brief-section-title">themes</h4><ul class="brief-list">${themes.map((t) => `<li><strong>${esc(t.name || "")}</strong><p>${esc(t.why || "")}</p>${t.refs ? `<div class="inspire-facet-refs">${inspRefChips(t.refs, refs)}</div>` : ""}</li>`).join("")}</ul>`
    : "";

  const palHtml = pal.length
    ? `<h4 class="brief-section-title">palette</h4><div class="inspire-swatches">${pal.map((p) => {
        const hex = (p.hex || "#888").toLowerCase();
        const ink = contrastText(hex);
        return `<button type="button" class="swatch" data-hex="${escAttr(p.hex || "")}" style="background:${escAttr(hex)};color:${ink}" title="copy ${escAttr(p.hex || "")}">
        <span class="swatch-name">${esc(p.name || "")}</span>
        <span class="swatch-role">${esc(p.role || "")}</span>
        <span class="swatch-hex">${esc(hex)}</span></button>`;
      }).join("")}</div>`
    : "";

  const typeHtml = (fams.length || (ts && ts.scale_note))
    ? `<h4 class="brief-section-title">type</h4>${fams.length ? `<div class="inspire-families">${fams.map((f) => {
        const fam = String(f.family || "").trim();
        const specimen = familySpecimenUrl(fam);
        const famStyle = cssFamilyString(fam);
        const nameEl = specimen
          ? `<a class="family-name-link" style="font-family:${famStyle}" href="${escAttr(specimen)}" target="_blank" rel="noopener">${esc(fam || "untitled")} ↗</a>`
          : `<span class="family-name" style="font-family:${famStyle}">${esc(fam || "untitled")}</span>`;
        const match = f.match ? `<span class="family-role">${esc(f.match)}</span>` : "";
        return `<div class="inspire-family" style="font-family:${famStyle}"><div class="family-meta">${nameEl}<span class="family-role">${esc(f.role || "body")}</span>${match}</div><p class="family-why">${esc(f.why || "")}</p></div>`;
      }).join("")}</div>` : ""}${ts && ts.scale_note ? `<p class="inspire-text">${esc(ts.scale_note)}</p>` : ""}`
    : "";

  const buildHtml = (build.shape || parts.length)
    ? `<h4 class="brief-section-title">build</h4>${build.shape ? `<p class="inspire-text">${esc(build.shape)}</p>` : ""}${parts.length ? `<ul class="brief-list">${parts.map((p) => `<li><strong>${esc(p.name || "")}</strong><p>${esc(p.what || "")}</p>${p.refs ? `<div class="inspire-facet-refs">${inspRefChips(p.refs, refs)}</div>` : ""}</li>`).join("")}</ul>` : ""}`
    : "";

  const makeHtml = make.length
    ? `<h4 class="brief-section-title">make</h4><ol class="brief-steps">${make.map((s) => `<li>${esc(s.step || "")}${s.detail ? `<em>${esc(s.detail)}</em>` : ""}</li>`).join("")}</ol>`
    : "";

  const stealHtml = steal.length
    ? `<h4 class="brief-section-title">steal</h4><ul class="brief-steal">${steal.map((s) => `<li>${esc(s)}</li>`).join("")}</ul>`
    : "";

  const promptHtml = b.build_prompt
    ? `<h4 class="brief-section-title">build prompt</h4><div class="brief-prompt" id="brief-prompt-box"><button type="button" class="brief-copy" id="brief-copy">copy</button>${esc(b.build_prompt)}</div>`
    : "";

  const refsHtml = refs.length
    ? '<h4 class="brief-section-title">from your vault</h4><div class="inspire-refs">' + refs.map((c, i) => '<div class="brief-ref"><span class="brief-ref-id">' + esc(c.ref || ("r" + i)) + "</span>" + similarCardHtml(c, i) + "</div>").join("") + '</div><p class="inspire-refs-note">click any card to open it.</p>'
    : "";

  const brainstorm = result.brainstorm && result.brainstorm.name
    ? `<p class="inspire-refs-note">open brainstorm · lens: ${esc(result.brainstorm.name)}</p>`
    : "";

  host.innerHTML = `
  <div class="inspire-output">
    <div class="inspire-head"><h3>${esc(b.title || "build brief")}</h3></div>
    ${b.kind ? `<span class="brief-kind">${esc(b.kind)}</span>` : ""}
    ${b.one_liner ? `<p class="brief-one-liner">${esc(b.one_liner)}</p>` : ""}
    ${themesHtml}
    ${palHtml}
    ${typeHtml}
    ${buildHtml}
    ${makeHtml}
    ${stealHtml}
    ${promptHtml}
    ${refsHtml}
    ${brainstorm}
  </div>`;
  host.classList.remove("hidden");

  const copyBtn = $("inspire-copy");
  if (copyBtn) copyBtn.classList.add("hidden");
  const exportBtn = $("inspire-export");
  if (exportBtn) exportBtn.classList.add("hidden");

  const pcopy = $("brief-copy");
  if (pcopy) pcopy.addEventListener("click", async () => {
    try {
      await copyText(b.build_prompt || "");
      pcopy.classList.add("copied");
      pcopy.textContent = "copied";
      setTimeout(() => { pcopy.classList.remove("copied"); pcopy.textContent = "copy"; }, 1200);
    } catch (_) {}
  });
  host.querySelectorAll(".swatch").forEach((sw) => sw.addEventListener("click", () => {
    const hex = sw.dataset.hex || "";
    if (!hex) return;
    copyText(hex).then(() => {
      sw.classList.add("copied");
      setTimeout(() => sw.classList.remove("copied"), 700);
    }).catch(() => {});
  }));
  host.querySelectorAll(".inspire-ref-chip").forEach((chip) => chip.addEventListener("click", () => {
    const id = chip.dataset.refId;
    if (id) openDetail(id);
  }));
  bindSimilarCards(host);
  hydrateSimilarMedia(host, refs, null);
}

function renderInspireResult(result) {
  const host = $("inspire-results");
  if (!host) return;
  const direction = result.direction || {};
  const candidates = result.candidates || [];
  const palette = Array.isArray(direction.palette) ? direction.palette : [];
  const guidance = result.palette_guidance || {};
  const guidanceWarnings = Array.isArray(guidance.warnings) ? guidance.warnings : [];
  const guidanceHtml = guidanceWarnings.length
    ? `<p class="inspire-palette-note">palette note: ${guidanceWarnings.map((warning) => esc(warning)).join(" ")}</p>`
    : "";
  const typo = direction.typography || {};
  const families = Array.isArray(typo.families) ? typo.families : [];
  const tags = Array.isArray(direction.tags) ? direction.tags : Array.isArray(direction.mood) ? direction.mood : [];
  const refIds = (direction.refs || []).map((t) => resolveInspRef(t, candidates)).filter(Boolean);
  // Show the full sampled mix (not just the cited refs) — more material to browse.
  const refs = candidates;

  const tagsHtml = tags.length ? `<div class="inspire-tags">${tags.map((t) => `<span class="chip-tag">${esc(t)}</span>`).join("")}</div>` : "";

  const palHtml = palette.length ? `<h4>palette</h4>${guidanceHtml}<div class="inspire-swatches">${palette.map((p) => {
    const hex = (p.hex || "#888").toLowerCase();
    const ink = contrastText(hex);
    return `
    <button type="button" class="swatch" data-hex="${esc(p.hex || "")}" style="background:${esc(hex)};color:${ink}" title="copy ${esc(p.hex || "")}">
      <span class="swatch-name">${esc(p.name || "")}</span>
      <span class="swatch-role">${esc(p.role || "")}</span>
      <span class="swatch-hex">${esc(hex)}</span>
    </button>`;
  }).join("")}</div>` : "";

  // Live font preview: load each unique family from Google Fonts and render a specimen.
  const uniqueFamilies = [];
  families.forEach((f) => {
    const fam = String(f.family || "").trim();
    if (fam && !uniqueFamilies.includes(fam)) uniqueFamilies.push(fam);
  });
  uniqueFamilies.forEach(loadGoogleFont);

  const typoHtml = (typo.direction || families.length) ? `<h4>typography</h4>${typo.direction ? `<p class="inspire-text">${esc(typo.direction)}</p>` : ""}${families.length ? `<div class="inspire-families">${families.map((f) => {
    const fam = String(f.family || "").trim();
    const specimen = familySpecimenUrl(fam);
    const famStyle = cssFamilyString(fam);
    // One name only, and it IS the link. The whole card renders in the font
    // so the user reads the actual face instead of a separate specimen.
    const famEl = specimen
      ? `<a class="family-name-link" style="font-family:${famStyle}" href="${esc(specimen)}" target="_blank" rel="noopener" title="open ${esc(fam)} on Google Fonts">${esc(fam || "Untitled")} ↗</a>`
      : `<span class="family-name" style="font-family:${famStyle}">${esc(fam || "Untitled")}</span>`;
    return `
    <div class="inspire-family" style="font-family:${famStyle}">
      <div class="family-meta">${famEl}<span class="family-role">${esc(f.role || "body")}</span></div>
      <p class="family-why">${esc(f.why || "")}</p>
    </div>`;
  }).join("")}</div>` : ""}` : "";

  // One cohesive material language (new) with legacy treatments fallback.
  const materials = direction.materials || direction.material_language || null;
  const legacyTreatments = Array.isArray(direction.treatments) ? direction.treatments : [];
  let materialsHtml = "";
  if (materials && (materials.name || materials.concept || (materials.facets || []).length)) {
    const facets = Array.isArray(materials.facets) ? materials.facets : [];
    materialsHtml = `<h4>materials · one language</h4>
      <div class="inspire-material">
        ${materials.name ? `<span class="material-name">${esc(materials.name)}</span>` : ""}
        ${materials.concept ? `<p class="material-concept">${esc(materials.concept)}</p>` : ""}
        ${facets.length ? `<ul class="inspire-treatments">${facets.map((x) => `<li><strong>${esc(x.name || "")}</strong><p>${esc(x.what || "")}</p><div class="inspire-facet-refs">${inspRefChips(x.refs || x.ref, candidates)}</div></li>`).join("")}</ul>` : ""}
      </div>`;
  } else if (legacyTreatments.length) {
    materialsHtml = `<h4>materials · treatments</h4><ul class="inspire-treatments">${legacyTreatments.map((x) => `<li><strong>${esc(x.name || "")}</strong><p>${esc(x.what || "")}</p><div class="inspire-facet-refs">${inspRefChips(x.refs || x.ref, candidates)}</div></li>`).join("")}</ul>`;
  }

  const threshold = result.threshold != null ? Number(result.threshold) : 0;
  const refsHtml = refs.length ? `<h4>drawn from your vault — ${refs.length} at similarity ≥ ${threshold.toFixed(2)}</h4><div class="inspire-refs">${refs.map((c, i) => similarCardHtml(c, i)).join("")}</div><p class="inspire-refs-note">all of these are your saved cards — click one to open it.</p>` : "";

  host.innerHTML = `
  <div class="inspire-output">
    <div class="inspire-head">
      <h3>${esc(direction.title || "direction")}</h3>
    </div>
    ${direction.summary ? `<p class="inspire-summary">${esc(direction.summary)}</p>` : ""}
    ${tagsHtml}
    ${palHtml}
    ${typoHtml}
    ${materialsHtml}
    ${refsHtml}
  </div>`;
  const tools = $("inspire-tools");
  const actions = $("inspire-tool-actions");
  const head = host.querySelector(".inspire-head");
  if (head && actions) head.appendChild(actions);
  if (tools) tools.classList.add("hidden");
  host.classList.remove("hidden");
  host.querySelectorAll(".swatch").forEach((sw) => sw.addEventListener("click", () => {
    const hex = sw.dataset.hex || "";
    if (!hex) return;
    copyText(hex).then(() => {
      sw.classList.add("copied");
      setTimeout(() => sw.classList.remove("copied"), 700);
    }).catch(() => {});
  }));
  host.querySelectorAll(".inspire-ref-chip").forEach((b) => b.addEventListener("click", () => {
    const id = b.dataset.refId;
    if (id) openDetail(id);
  }));
  const copyBtn = $("inspire-copy");
  if (copyBtn) {
    copyBtn.classList.remove("hidden");
    copyBtn.onclick = async () => {
      copyBtn.disabled = true;
      try {
        await copyText(buildInspireCopy(direction));
        copyBtn.classList.add("copied");
        copyBtn.title = "direction copied";
        copyBtn.setAttribute("aria-label", "direction copied");
        setTimeout(() => {
          copyBtn.classList.remove("copied");
          copyBtn.title = "copy direction";
          copyBtn.setAttribute("aria-label", "copy direction");
        }, 1400);
      } catch (err) {
        copyBtn.classList.add("copy-failed");
        copyBtn.title = err.message || "copy failed";
        copyBtn.setAttribute("aria-label", err.message || "copy failed");
        setTimeout(() => {
          copyBtn.classList.remove("copy-failed");
          copyBtn.title = "copy direction";
          copyBtn.setAttribute("aria-label", "copy direction");
        }, 1800);
      } finally {
        copyBtn.disabled = false;
      }
    };
  }
  bindSimilarCards(host);
  hydrateSimilarMedia(host, refs, null);
}

/* ---------- inspire preview: render the direction as images ---------- */
let inspPreviewCtx = null; // { inspireJobId, direction } of the shown result

function resetInspirePreview() {
  inspPreviewCtx = null;
  const wrap = $("inspire-preview");
  if (wrap) wrap.classList.add("hidden");
  const q = $("inspire-preview-q");
  if (q) q.value = "";
  const exportBtn = $("inspire-export");
  if (exportBtn) {
    exportBtn.classList.add("hidden");
    exportBtn.disabled = false;
    exportBtn.title = "download this page as a self-contained HTML file";
  }
  const status = $("inspire-preview-status");
  if (status) {
    status.className = "import-status";
    status.textContent = "";
  }
  const shots = $("inspire-preview-shots");
  if (shots) shots.innerHTML = "";
}

function renderInspirePreviewComposer(result, inspireJobId) {
  inspPreviewCtx = {
    inspireJobId: inspireJobId || "",
    direction: result.direction || {},
  };
  const wrap = $("inspire-preview");
  if (wrap) wrap.classList.remove("hidden");
}

function previewShotHtml(name, variantLabel) {
  return `
  <figure class="preview-shot" data-media="${escHtmlAttr(name)}">
    <div class="preview-shot-img" data-ph><span class="preview-shot-pending">${esc(variantLabel)}</span></div>
    <button type="button" class="shot-dl" title="download this render" aria-label="download this render"><span class="ic"><svg viewBox="0 0 24 24"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="3" x2="12" y2="15"/></svg></span></button>
  </figure>`;
}

function downloadShot(fig) {
  const name = fig.dataset.media || "";
  if (!name) return;
  authedBlob(name).then((u) => {
    const a = document.createElement("a");
    a.href = u;
    a.download = name;
    // body-attached clicks bubble to the document outside-click handler,
    // which would close the whole inspire view. Stay inside #inspire and
    // stop the synthetic click from bubbling at all.
    a.addEventListener("click", (e) => e.stopPropagation());
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 4000);
  }).catch(() => {});
}

async function runInspirePreview() {
  const q = $("inspire-preview-q");
  const status = $("inspire-preview-status");
  const shots = $("inspire-preview-shots");
  const btn = $("inspire-preview-btn");
  const subject = (q && q.value || "").trim();
  if (!subject) {
    if (q) q.focus();
    return;
  }
  if (!inspPreviewCtx || !inspPreviewCtx.direction || !Object.keys(inspPreviewCtx.direction).length) {
    if (status) {
      status.className = "import-status error";
      status.textContent = "run inspire first — there is no direction to apply";
    }
    return;
  }
  if (status) {
    status.className = "import-status";
    status.textContent = "rendering 3 previews of your subject in this direction…";
  }
  if (shots) shots.innerHTML = "";
  // a fresh render should be visible even if the user collapsed the grid
  shots.classList.remove("collapsed");
  const toggleBtn = $("inspire-preview-toggle");
  if (toggleBtn) toggleBtn.setAttribute("aria-expanded", "true");
  if (btn) btn.disabled = true;
  const restoreBtn = () => { if (btn) btn.disabled = false; };
  try {
    const started = await api("/api/inspire/preview", {
      method: "POST",
      body: {
        inspire_job_id: inspPreviewCtx.inspireJobId,
        direction: inspPreviewCtx.direction,
        subject,
      },
    });
    const phase = { queued: "queued…", working: "rendering…" };
    while (true) {
      await new Promise((resolve) => setTimeout(resolve, 1600));
      let job;
      try {
        job = await api("/api/inspire/preview/" + encodeURIComponent(started.job_id));
      } catch (err) {
        if (err.message === "auth required") { restoreBtn(); return; }
        if (status) status.textContent = "polling failed: " + err.message;
        continue;
      }
      if (!job || job.state === "missing") {
        if (status) { status.className = "import-status error"; status.textContent = "preview job vanished"; }
        restoreBtn();
        return;
      }
      if (job.state === "error") {
        if (status) { status.className = "import-status error"; status.textContent = "preview failed: " + (job.error || "unknown"); }
        restoreBtn();
        return;
      }
      if (job.state === "done") {
        const imgs = (job.result && job.result.images) || [];
        if (status) { status.className = "import-status"; status.textContent = ""; }
        if (shots) {
          const variants = ["quiet variant", "material variant", "bold variant"];
          shots.innerHTML = imgs.map((n, i) => previewShotHtml(n, variants[i] || ("variant " + (i + 1)))).join("");
          for (const fig of shots.querySelectorAll(".preview-shot")) {
            const holder = fig.querySelector(".preview-shot-img");
            const name = fig.dataset.media || "";
            authedBlob(name).then((u) => {
              if (!holder.isConnected) return;
              const im = document.createElement("img");
              im.loading = "eager";
              im.alt = "design preview";
              im.addEventListener("load", () => {
                if (!holder.isConnected) return;
                holder.replaceWith(im);
                requestAnimationFrame(() => im.classList.add("is-ready"));
              }, { once: true });
              im.addEventListener("error", () => {
                if (holder.isConnected) holder.innerHTML = "<span class='preview-shot-pending'>render unavailable</span>";
              }, { once: true });
              im.src = u;
            }).catch(() => {
              if (holder.isConnected) holder.innerHTML = "<span class='preview-shot-pending'>render unavailable</span>";
            });
          }
          shots.querySelectorAll(".preview-shot .shot-dl").forEach((b) => {
            b.addEventListener("click", (e) => {
              e.preventDefault();
              const fig = e.target.closest(".preview-shot");
              if (fig) downloadShot(fig);
            });
          });
        }
        restoreBtn();
        return;
      }
      if (status) status.textContent = phase[job.state] || "rendering…";
    }
  } catch (err) {
    if (err.message !== "auth required" && status) {
      status.className = "import-status error";
      status.textContent = "preview failed: " + err.message;
    }
    restoreBtn();
  }
}

function initInspirePreview() {
  const btn = $("inspire-preview-btn");
  const q = $("inspire-preview-q");
  const exportBtn = $("inspire-export");
  const toggle = $("inspire-preview-toggle");
  if (btn) btn.addEventListener("click", (e) => { e.preventDefault(); runInspirePreview(); });
  if (q) q.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); runInspirePreview(); }
  });
  if (exportBtn) exportBtn.addEventListener("click", downloadInspireExport);
  if (toggle) toggle.addEventListener("click", (e) => {
    e.preventDefault();
    const shots = $("inspire-preview-shots");
    if (!shots) return;
    const collapsed = shots.classList.toggle("collapsed");
    toggle.setAttribute("aria-expanded", String(!collapsed));
    toggle.title = collapsed ? "show the renders" : "hide the renders";
  });
}

async function downloadInspireExport() {
  const btn = $("inspire-export");
  if (!btn || btn.disabled) return;
  if (!inspPreviewCtx || !inspPreviewCtx.inspireJobId) return;
  btn.disabled = true;
  btn.title = "building export…";
  try {
    const res = await fetch(serverBase() + "/api/inspire/" + encodeURIComponent(inspPreviewCtx.inspireJobId) + "/export", { headers: authHeader() });
    if (res.status === 401) { showLogin(); return; }
    if (!res.ok) throw new Error("export " + res.status);
    const blob = await res.blob();
    const cd = res.headers.get("Content-Disposition") || "";
    const mf = cd.match(/filename="([^"]+)"/);
    const u = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = u;
    a.download = mf ? mf[1] : "inspire-export.html";
    // see downloadShot: a body-attached click must not bubble to the
    // document outside-click handler (it would close the inspire view).
    a.addEventListener("click", (e) => e.stopPropagation());
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(u), 6000);
    btn.title = "export saved";
  } catch (err) {
    if (err && err.message) btn.title = err.message;
  } finally {
    btn.disabled = false;
  }
}

/* ---------- helpers ---------- */
function esc(s) {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
function hexToRgb(hex) {
  const value = String(hex || "").replace(/^#/, "");
  if (!/^[0-9a-f]{6}$/i.test(value)) return null;
  return [parseInt(value.slice(0, 2), 16), parseInt(value.slice(2, 4), 16), parseInt(value.slice(4, 6), 16)];
}

function rgbToHsvArr(rgb) {
  if (!rgb) return [0, 0, 0];
  const values = rgb.map((value) => value / 255);
  const max = Math.max(...values);
  const min = Math.min(...values);
  const delta = max - min;
  let hue = 0;
  if (delta) {
    if (max === values[0]) hue = 60 * (((values[1] - values[2]) / delta) % 6);
    else if (max === values[1]) hue = 60 * ((values[2] - values[0]) / delta + 2);
    else hue = 60 * ((values[0] - values[1]) / delta + 4);
  }
  if (hue < 0) hue += 360;
  const saturation = max ? delta / max : 0;
  return [Math.round(hue), Math.round(saturation * 100), Math.round(max * 100)];
}

function escAttr(s) {
  return String(s == null ? "" : s).replace(/[^a-z0-9_-]/gi, "-").replace(/^-+|-+$/g, "") || "link";
}

/* Real HTML attribute escaper. escAttr() is a slugify (used for ids/classes);
   never feed URLs through it — it turns "https://x.com/a" into "https---x-com-a". */
function escHtmlAttr(s) {
  return String(s == null ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/* Open an external URL only when it actually is http(s). A schemeless or
   slugified value would resolve as a path on the vault origin. */
function openExternalUrl(url) {
  if (url && /^https?:\/\//i.test(url)) {
    window.open(url, "_blank", "noopener");
    return true;
  }
  return false;
}

/* ---------- boot ---------- */
(async function boot() {
  // The whole file has evaluated by now, so ICONS/HUE_COLORS exist.
  syncGridSizeButton();
  initHuePopup();
  initFab();
  initRailHover();
  initInspirePreview();

  /* ---------- sidebar hover-expand (desktop) ---------- */
  function initRailHover() {
    const rail = document.querySelector(".topbar");
    if (!rail) return;
    rail.addEventListener("mouseenter", () => {
      document.body.classList.add("rail-hover");
      syncTopbarVar();
    });
    rail.addEventListener("mouseleave", () => {
      document.body.classList.remove("rail-hover");
      syncTopbarVar();
    });
    // The rail width transition is 220ms. When the rail expands/collapses the
    // visible field width changes; re-flow the active masonry after the
    // transition finishes so cards never keep a stale column layout.
    let railReflowTimer = 0;
    function scheduleRailReflow() {
      if (railReflowTimer) return;
      railReflowTimer = setTimeout(() => {
        railReflowTimer = 0;
        if (typeof $ != "function") return;
        if (isFollowupsOpen() && typeof fuMasonry === "function") fuMasonry();
        else if (typeof masonry === "function") masonry();
      }, 240);
    }
    rail.addEventListener("mouseenter", scheduleRailReflow);
    rail.addEventListener("mouseleave", scheduleRailReflow);
  }

  /* ---------- FAB command overlay (experiment) ---------- */
  function initFab() {
    const fab = $("fab");
    const overlay = $("cmd-overlay");
    const close = $("cmd-close");
    const slot = $("cmd-panel-slot");
    if (!fab || !overlay) return;

    // Hidden parking container. Elements moved out of the slot land here,
    // NOT detached from the document — getElementById must keep finding
    // them, or unrelated handlers (cleanup/openCleanupView etc.) throw.
    const park = document.createElement("div");
    park.id = "cmd-park";
    park.style.display = "none";
    document.body.appendChild(park);

    function openOverlay() {
      overlay.classList.remove("hidden");
      const input = $("cmd-search-q");
      if (input) setTimeout(() => input.focus(), 50);
    }
    function closeOverlay() {
      overlay.classList.add("hidden");
      clearSlot();
    }
    function clearSlot() {
      while (slot && slot.firstChild) park.appendChild(slot.firstChild);
    }
    function parkEl(el) {
      if (el && el.parentElement) park.appendChild(el);
    }

    fab.addEventListener("click", openOverlay);
    // Quick-action fan: hover the FAB, click a mini button → open overlay
    // and trigger the matching command chip.
    document.querySelectorAll(".fab-mini").forEach((mini) => {
      mini.addEventListener("click", (e) => {
        e.stopPropagation(); // keep the global outside-click handler from
        // instantly closing the view this fan button just opened
        const target = $(mini.dataset.cmd);
        if (!target) return;
        openOverlay();
        target.click();
      });
    });
    close.addEventListener("click", closeOverlay);
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) closeOverlay();
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && !overlay.classList.contains("hidden")) {
        closeOverlay();
      }
    });

    // Search: submit, close overlay, results show in background
    $("cmd-search-form").addEventListener("submit", (e) => {
      e.preventDefault();
      const q = $("cmd-search-q").value.trim();
      $("search-q").value = q;
      state.query = q;
      state.offset = 0;
      refresh({ replace: true });
      closeOverlay();
    });

    // Inspire: close overlay, run inspire in background
    $("cmd-inspire").addEventListener("click", () => {
      const q = $("cmd-search-q").value.trim();
      $("search-q").value = q;
      closeOverlay();
      $("sort-menu").classList.add("hidden");
      $("filters").classList.add("collapsed");
      closeCleanupView();
      openInspireView();
      runInspire(q);
    });

    // Filters: toggle inline in the panel slot (don't close overlay)
    $("cmd-filters").addEventListener("click", (e) => {
      e.stopPropagation();
      const filters = $("filters");
      const inSlot = slot && slot.contains(filters);
      if (inSlot) {
        filters.classList.add("collapsed");
        parkEl(filters);
      } else {
        filters.classList.remove("collapsed");
        clearSlot();
        slot.appendChild(filters);
      }
    });

    // Sort: toggle inline in the panel slot
    $("cmd-sort").addEventListener("click", (e) => {
      e.stopPropagation();
      const menu = $("sort-menu");
      const inSlot = slot && slot.contains(menu);
      if (inSlot) {
        menu.classList.add("hidden");
        parkEl(menu);
      } else {
        clearSlot();
        slot.appendChild(menu);
        menu.classList.remove("hidden");
        syncSortUI();
      }
    });

    // Grid size: cycle in place, don't close overlay
    $("cmd-gridsize").addEventListener("click", (e) => {
      e.stopPropagation();
      const preset = gridSizePreset(settings.gridSize);
      const next = GRID_ORDER[(GRID_ORDER.indexOf(preset) + 1) % GRID_ORDER.length];
      settings.gridSize = GRID_SIZE_PRESETS[next] || GRID_SIZE.default;
      saveSettings();
      syncGridSizeButton();
      // update the chip icon inline
      const chip = $("cmd-gridsize");
      if (chip) {
        const icWrap = chip.querySelector(".ic");
        if (icWrap) icWrap.innerHTML = ICONS["grid-" + next] || ICONS["grid-regular"];
      }
      masonry();
    });

    // Posts: close overlay, back to the vault grid (mobile has no sidebar —
    // this is the "home" escape hatch from inside a full-page view)
    $("cmd-posts").addEventListener("click", (e) => {
      e.stopPropagation();
      closeOverlay();
      closeDetail();
      if (isFollowupsOpen()) closeFollowupsView();
      if (isTrashOpen()) closeTrashView();
      if (isGroupsOpen()) closeGroupsView();
      if (!$("cleanup").classList.contains("hidden")) closeCleanupView();
      if (!$("settings").classList.contains("hidden")) { $("settings").classList.add("hidden"); pushView("posts"); }
      if ($("inspire") && !$("inspire").classList.contains("hidden")) { closeInspireView(); pushView("posts"); }
      $("filters").classList.add("collapsed");
    });

    // Follow-ups: close overlay, open follow-ups view
    $("cmd-followups").addEventListener("click", (e) => {
      e.stopPropagation();
      closeOverlay();
      if (!isFollowupsOpen()) openFollowupsView();
    });

    // Trash: close overlay, open trash view
    $("cmd-trash").addEventListener("click", (e) => {
      e.stopPropagation();
      closeOverlay();
      if (!isTrashOpen()) openTrashView();
    });

    // Cleanup: close overlay, open cleanup view
    $("cmd-cleanup").addEventListener("click", (e) => {
      e.stopPropagation();
      closeOverlay();
      const view = $("cleanup");
      if (view.classList.contains("hidden")) openCleanupView();
    });

    // Groups: close overlay, open groups view
    $("cmd-groups").addEventListener("click", (e) => {
      e.stopPropagation();
      closeOverlay();
      const view = $("groups");
      if (view.classList.contains("hidden")) openGroupsView();
    });

    // Settings: close overlay, open settings (never hide the FAB — the page
    // is left via the sidebar like the other full pages)
    $("cmd-settings").addEventListener("click", (e) => {
      e.stopPropagation();
      closeOverlay();
      $("settings").classList.remove("hidden");
      syncSettingsUI();
      pushView("settings");
    });
  }

  const saved = new URLSearchParams(location.search).get("saved");
  const stored = localStorage.getItem(AUTH_KEY);
  if (stored) {
    try {
      state.creds = JSON.parse(stored);
      const meta = await api("/api/meta");
      const settingsUser = $("settings-username");
      if (settingsUser) settingsUser.textContent = state.creds.user;
      showMain();
      hydrateMeta(meta);
      await loadServerSettings();
      syncSortUI();
      // The browser can restore old form text after a reload. The landing
      // feed starts without a query, so keep the field consistent with it.
      $("search-q").value = state.query;
      await refresh({ replace: true });
      if (!saved) restoreView();
      if (saved) openDetail(saved);
      return;
    } catch (e) {
      localStorage.removeItem(AUTH_KEY);
      state.creds = null;
    }
  }
  showLogin();
})();

/* ---------- TEMPORARY client debug beacon (remove after grid-size fix) ---------- */
function dbgMark(ev, extra) {
  try {
    const K = "tv-debug-log";
    const a = JSON.parse(localStorage.getItem(K) || "[]");
    a.push(Object.assign({ t: Date.now(), e: ev }, extra || {}));
    localStorage.setItem(K, JSON.stringify(a.slice(-50)));
  } catch (_) {}
}
dbgMark("script-end", { href: location.href });
window.addEventListener("error", (e) => {
  dbgMark("jserr", { msg: String(e.message).slice(0, 200), src: String(e.filename).slice(-40), line: e.lineno, stack: String(e.error && e.error.stack || "").slice(0, 300) });
});
window.addEventListener("unhandledrejection", (e) => {
  dbgMark("jsrej", { msg: String(e.reason && e.reason.message || e.reason).slice(0, 200), stack: String(e.reason && e.reason.stack || "").slice(0, 300) });
});
document.addEventListener("click", (e) => {
  const btn = $("btn-gridsize");
  if (!btn) { dbgMark("click-nobtn"); return; }
  const path = e.composedPath ? e.composedPath() : [];
  if (!path.includes(btn) && !(e.target && e.target.closest && e.target.closest("#btn-gridsize"))) return;
  const hit = document.elementFromPoint(e.clientX, e.clientY);
  const chain = [];
  let n = hit;
  for (let i = 0; i < 4 && n; i++) { chain.push(n.id || n.tagName + "." + String(n.className).slice(0, 40)); n = n.parentElement; }
  dbgMark("btn-click-capture", { trusted: e.isTrusted, x: Math.round(e.clientX), y: Math.round(e.clientY), hit: chain.join(" < "), gridSize: settings.gridSize, title: btn.title, rect: JSON.stringify(btn.getBoundingClientRect()) });
  setTimeout(() => {
    const card = document.querySelector("#grid .card[data-id]");
    dbgMark("after-click", { gridSize: settings.gridSize, title: btn.title, cardW: card ? Math.round(card.getBoundingClientRect().width) : -1, cards: document.querySelectorAll("#grid .card[data-id]").length });
  }, 120);
}, true);